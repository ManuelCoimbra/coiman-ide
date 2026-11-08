#!/usr/bin/env escript
%% Between Coiman IDE and a running Erlang or Elixir node, joined as a hidden node.
%% Reads commands on stdin and writes events on stdout, a line each, with text in
%% base64. Code runs in a process of its own on that node, so it can be stopped.

main([Node | Rest]) ->
    Target = list_to_atom(Node),
    Mode = case lists:member($., Node) of true -> longnames; false -> shortnames end,
    Name = list_to_atom("coiman_" ++ os:getpid()),
    case net_kernel:start([Name, Mode]) of
        {ok, _} -> ok;
        {error, Why} -> fatal(io_lib:format("Can't join as a node: ~tp. Is epmd running?", [Why]))
    end,
    case Rest of
        [Cookie | _] -> erlang:set_cookie(Target, list_to_atom(Cookie));
        [] -> ok
    end,
    case net_kernel:hidden_connect_node(Target) of
        true -> ok;
        _ -> fatal(io_lib:format("Can't reach ~s: is it running, with this cookie?", [Node]))
    end,
    monitor_node(Target, true),
    Elixir = rpc:call(Target, code, which, ['Elixir.Code']) =/= non_existing,
    Otp = rpc:call(Target, erlang, system_info, [otp_release]),
    System = case Elixir of
        true -> io_lib:format("~s, Elixir ~s, OTP ~s", [Node, rpc:call(Target, 'Elixir.System', version, []), Otp]);
        false -> io_lib:format("~s, OTP ~s", [Node, Otp])
    end,
    say(["HELLO ", b64(System), " eval,inspect,interrupt,stdin"]),
    Main = self(),
    spawn_link(fun() -> reader(Main) end),
    loop(#{node => Target, elixir => Elixir, running => #{}, input => none,
           bindings => #{erlang => erl_eval:new_bindings(), elixir => []}}).

fatal(Text) ->
    say(["PROBLEM ", b64(Text)]),
    halt(3).

b64(Text) ->
    base64:encode(unicode:characters_to_binary(Text)).

unb64(Text) ->
    base64:decode(Text).

say(Words) ->
    io:put_chars(standard_io, [Words, "\n"]).

reader(Main) ->
    case io:get_line(standard_io, "") of
        eof -> Main ! stop;
        {error, _} -> Main ! stop;
        Line -> Main ! {line, string:lexemes(string:trim(Line), " ")}, reader(Main)
    end.

loop(State = #{node := Node, running := Running}) ->
    receive
        stop -> halt(0);
        {nodedown, Node} -> say(["PROBLEM ", b64("The node went down")]), halt(0);
        {line, ["EVAL", Id, Language, Code]} ->
            case start_eval(self(), Id, Language, unb64(list_to_binary(Code)), State) of
                none -> loop(State);
                Pid -> loop(State#{running := Running#{Pid => Id}})
            end;
        {line, ["INSPECT", Id | Query]} ->
            spawn(fun() -> say(["ROWS ", Id, " ", b64(rows(Node, Query))]) end),
            loop(State);
        {line, ["STDIN", Text]} ->
            case maps:get(input, State) of
                none -> ok;
                Reader -> Reader ! {answer, unb64(list_to_binary(Text))}
            end,
            loop(State#{input := none});
        {line, ["INTERRUPT"]} ->
            [exit(P, kill) || P <- maps:keys(Running)],
            loop(State);
        {line, _} -> loop(State);
        {waiting_input, Reader, Id} ->
            say(["INPUT ", Id]),
            loop(State#{input := Reader});
        {'DOWN', _, process, Pid, Why} ->
            case maps:take(Pid, Running) of
                {Id, Left} ->
                    %% The result comes first, so anything else here means it never will.
                    receive {rc_result, Id, _, _} = R -> self() ! R after 300 -> ok end,
                    case Why of
                        normal -> ok;
                        killed -> say(["FAIL ", Id, " ", b64("Interrupted")]), say(["DONE ", Id]);
                        _ -> say(["FAIL ", Id, " ", b64(io_lib:format("~tp", [Why]))]), say(["DONE ", Id])
                    end,
                    loop(State#{running := Left});
                error -> loop(State)
            end;
        {rc_result, Id, Use, {ok, Text, Kept}} ->
            say(["VALUE ", Id, " ", b64(Text)]), say(["DONE ", Id]),
            loop(State#{bindings := (maps:get(bindings, State))#{Use := Kept}});
        {rc_result, Id, _, {error, Text}} ->
            say(["FAIL ", Id, " ", b64(Text)]), say(["DONE ", Id]), loop(State)
    end.

%% Runs on the node with nothing of ours loaded there: the code is its own wrapping.
%% Variables come back with the result, so the next send can use them.
start_eval(Main, Id, Language, Code, State = #{node := Node, elixir := Elixir}) ->
    Use = case Language of
        "elixir" -> elixir;
        "erlang" -> erlang;
        _ when Elixir -> elixir;
        _ -> erlang
    end,
    Old = maps:get(Use, maps:get(bindings, State)),
    Prepared = case Use of
        elixir ->
            {ok, Code,
             "R = try {V, B} = 'Elixir.Code':eval_string(Src, Old, []), {ok, 'Elixir.Kernel':inspect(V), B} "
             "catch C:E:S -> {error, 'Elixir.Exception':format(C, E, S)} end, Parent ! {rc_result, Id, Use, R}."};
        erlang ->
            Body = string:trim(string:trim(unicode:characters_to_list(Code)), trailing, "."),
            case parse(Body ++ ".") of
                {ok, UserForms} ->
                    {ok, UserForms,
                     "R = try {value, V, B} = erl_eval:exprs(Src, Old), {ok, io_lib:format(\"~tp\", [V]), B} "
                     "catch C:E -> {error, io_lib:format(\"~tp: ~tp\", [C, E])} end, Parent ! {rc_result, Id, Use, R}."};
                Failed -> Failed
            end
    end,
    case Prepared of
        {error, Why} ->
            Main ! {rc_result, Id, Use, {error, Why}},
            none;
        {ok, Source, Wrapper} ->
            {ok, Forms} = parse(Wrapper),
            Bindings = lists:foldl(
                fun({K, V}, B) -> erl_eval:add_binding(K, V, B) end,
                erl_eval:new_bindings(),
                [{'Src', Source}, {'Old', Old}, {'Parent', Main}, {'Id', Id}, {'Use', Use}]),
            Io = spawn(fun() -> io_server(Main, Id) end),
            %% A spawned process's output goes to the group leader of who spawned it.
            Starter = spawn(fun() ->
                group_leader(Io, self()),
                {Pid, Ref} = spawn_monitor(Node, erl_eval, exprs, [Forms, Bindings]),
                Main ! {started, self(), Pid},
                receive {'DOWN', Ref, process, Pid, Why} -> Main ! {'DOWN', Ref, process, Pid, Why} end
            end),
            receive {started, Starter, Remote} -> Remote end
    end.

parse(Text) ->
    case erl_scan:string(Text) of
        {ok, Tokens, _} ->
            case erl_parse:parse_exprs(Tokens) of
                {ok, Forms} -> {ok, Forms};
                {error, {_, Module, What}} -> {error, Module:format_error(What)}
            end;
        {error, {_, Module, What}, _} -> {error, Module:format_error(What)}
    end.

io_server(Main, Id) ->
    receive
        {io_request, From, Reply, Request} ->
            Answer = io_request(Main, Id, Request),
            From ! {io_reply, Reply, Answer},
            io_server(Main, Id)
    end.

io_request(_, Id, {put_chars, _, Chars}) -> say(["OUT ", Id, " ", b64(Chars)]), ok;
io_request(_, Id, {put_chars, _, M, F, A}) -> say(["OUT ", Id, " ", b64(apply(M, F, A))]), ok;
io_request(Main, Id, {get_line, _, _}) -> read(Main, Id);
io_request(Main, Id, {get_until, _, _, _, _, _}) -> read(Main, Id);
io_request(Main, Id, {get_chars, _, _, _}) -> read(Main, Id);
io_request(_, _, {requests, _}) -> ok;
io_request(_, _, {setopts, _}) -> ok;
io_request(_, _, getopts) -> [];
io_request(_, _, _) -> {error, request}.

read(Main, Id) ->
    Main ! {waiting_input, self(), Id},
    receive {answer, Text} -> binary_to_list(Text) end.

%% Named processes on the node, and what's known of one when it's opened.
rows(Node, []) ->
    case rpc:call(Node, erlang, registered, []) of
        Names when is_list(Names) ->
            [row(Node, N) || N <- lists:sort(Names)];
        _ -> ""
    end;
rows(Node, [Query]) ->
    Name = binary_to_atom(unb64(list_to_binary(Query))),
    case rpc:call(Node, erlang, whereis, [Name]) of
        Pid when is_pid(Pid) ->
            Info = rpc:call(Node, erlang, process_info, [Pid]),
            [line(atom_to_list(K), "", type_of(V), show(V)) || {K, V} <- Info, is_list(Info)];
        _ -> ""
    end.

row(Node, Name) ->
    Pid = rpc:call(Node, erlang, whereis, [Name]),
    Info = rpc:call(Node, erlang, process_info, [Pid, [status, message_queue_len, memory]]),
    case Info of
        [{status, S}, {message_queue_len, Q}, {memory, M}] ->
            Text = io_lib:format("~s, ~b waiting, ~b bytes", [S, Q, M]),
            line(atom_to_list(Name), atom_to_list(Name), "process", Text);
        _ -> ""
    end.

type_of(V) when is_list(V) -> "list";
type_of(V) when is_atom(V) -> "atom";
type_of(V) when is_integer(V) -> "integer";
type_of(V) when is_pid(V) -> "pid";
type_of(V) when is_tuple(V) -> "tuple";
type_of(_) -> "term".

show(V) ->
    Text = lists:flatten(io_lib:format("~tp", [V])),
    Flat = [case C of $\n -> $\s; $\t -> $\s; _ -> C end || C <- Text],
    string:slice(Flat, 0, 300).

line(Name, Child, Type, Value) ->
    ["@@row@@\t", Name, "\t", Child, "\t", Type, "\t", Value, "\n"].
