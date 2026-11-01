"""Between Rust Code and a Jupyter kernel, whose wire protocol needs ZeroMQ.

Reads commands on stdin and writes events on stdout, a line each, with text in
base64. With a connection file it attaches to a kernel already running, and leaves
it running. With none it starts one, which ends with this program.
"""
import base64
import os
import queue
import sys
import threading

try:
    from jupyter_client import BlockingKernelClient, KernelManager
except ImportError:
    sys.stdout.write("PROBLEM " + base64.b64encode(
        b"This Python has no jupyter_client: pip install ipykernel jupyter_client").decode() + "\n")
    sys.stdout.flush()
    sys.exit(3)

INSPECT = r'''
def _row(name, child, value):
    one = lambda s: " ".join(str(s).split())
    text = repr(value)
    text = text[:300] + "..." if len(text) > 300 else text
    print("@@row@@\t%s\t%s\t%s\t%s" % (one(name), one(child), one(type(value).__name__), one(text)))

def _opens(v):
    return isinstance(v, (dict, list, tuple, set, frozenset)) and len(v) > 0 or (hasattr(v, "__dict__") and not callable(v) and not isinstance(v, type) and type(v).__name__ != "module")

def _kids(e, v):
    if isinstance(v, dict):
        return [(repr(k), "%s[%r]" % (e, k), x) for k, x in list(v.items())[:200]]
    if isinstance(v, (list, tuple)):
        return [(str(i), "%s[%d]" % (e, i), x) for i, x in enumerate(v[:200])]
    if isinstance(v, (set, frozenset)):
        return [(str(i), "list(%s)[%d]" % (e, i), x) for i, x in enumerate(list(v)[:200])]
    return [(k, "%s.%s" % (e, k), x) for k, x in list(vars(v).items())[:200]]

if query is None:
    hidden = {"In", "Out", "exit", "quit", "get_ipython", "open"}
    items = [(k, k, v) for k, v in sorted(ns.items()) if not k.startswith("_") and k not in hidden]
    items = [i for i in items if type(i[2]).__name__ != "module"]
else:
    v = eval(query, ns)
    items = _kids(query, v)
for name, child, value in items:
    _row(name, child if _opens(value) else "", value)
'''


def b64(text):
    return base64.b64encode(text.encode("utf-8", "replace")).decode()


def unb64(text):
    return base64.b64decode(text).decode("utf-8", "replace")


class Bridge:
    def __init__(self, connection):
        self.lock = threading.Lock()
        self.requests = {}
        self.printed = {}
        self.unlisted = {}
        if connection:
            self.manager = None
            self.client = BlockingKernelClient(connection_file=connection)
            self.client.load_connection_file()
        else:
            self.manager = KernelManager()
            self.manager.start_kernel()
            self.client = self.manager.client()
        self.client.start_channels()
        self.client.wait_for_ready(timeout=60)

    def say(self, *words):
        with self.lock:
            sys.stdout.write(" ".join(str(w) for w in words) + "\n")
            sys.stdout.flush()

    def hello(self):
        msg = self.client.kernel_info(reply=True)["content"]
        info = msg.get("language_info", {})
        system = "%s %s (%s kernel %s)" % (
            info.get("name", "kernel"), info.get("version", ""),
            msg.get("implementation", "?"), msg.get("implementation_version", ""))
        abilities = "eval,inspect,stdin"
        if self.manager:
            abilities += ",interrupt"
        self.say("HELLO", b64(system), abilities)

    def iopub(self):
        while True:
            try:
                msg = self.client.get_iopub_msg(timeout=None)
            except Exception:
                os._exit(0)
            request = self.requests.get(msg["parent_header"].get("msg_id"))
            if not request:
                continue
            number, kind = request
            kinds, content = msg["msg_type"], msg["content"]
            if kind == "inspect":
                if kinds == "stream":
                    self.printed[number] = self.printed.get(number, "") + content["text"]
                elif kinds == "error":
                    self.unlisted[number] = "%s: %s" % (content["ename"], content["evalue"])
                elif kinds == "status" and content["execution_state"] == "idle":
                    if number in self.unlisted:
                        self.say("UNLISTED", number, b64(self.unlisted.pop(number)))
                    else:
                        self.say("ROWS", number, b64(self.printed.pop(number, "")))
                continue
            if kinds == "stream":
                self.say("ERR" if content["name"] == "stderr" else "OUT", number, b64(content["text"]))
            elif kinds in ("execute_result", "display_data"):
                text = content["data"].get("text/plain")
                if text:
                    self.say("VALUE", number, b64(text))
            elif kinds == "error":
                self.say("FAIL", number, b64("%s: %s" % (content["ename"], content["evalue"])))
            elif kinds == "status" and content["execution_state"] == "idle":
                self.say("DONE", number)

    def stdin_channel(self):
        while True:
            try:
                msg = self.client.get_stdin_msg(timeout=None)
            except Exception:
                os._exit(0)
            if msg["msg_type"] == "input_request":
                request = self.requests.get(msg["parent_header"].get("msg_id"))
                self.say("INPUT", request[0] if request else 0)

    def run(self, commands):
        threading.Thread(target=self.iopub, daemon=True).start()
        threading.Thread(target=self.stdin_channel, daemon=True).start()
        self.hello()
        while True:
            words = commands.get()
            if words is None:
                break
            op = words[0]
            if op == "EVAL":
                msg_id = self.client.execute(unb64(words[2]), allow_stdin=True)
                self.requests[msg_id] = (int(words[1]), "eval")
            elif op == "INSPECT":
                query = unb64(words[2]) if len(words) > 2 else None
                code = "exec(%r, {'ns': globals(), 'query': %r})" % (INSPECT, query)
                msg_id = self.client.execute(code, silent=True, store_history=False, allow_stdin=False)
                self.requests[msg_id] = (int(words[1]), "inspect")
            elif op == "STDIN":
                self.client.input(unb64(words[1]).rstrip("\r\n"))
            elif op == "INTERRUPT" and self.manager:
                self.manager.interrupt_kernel()
        self.client.stop_channels()
        if self.manager:
            self.manager.shutdown_kernel(now=True)


def main():
    commands = queue.Queue()

    def read():
        for line in sys.stdin:
            commands.put(line.split())
        commands.put(None)

    threading.Thread(target=read, daemon=True).start()
    try:
        bridge = Bridge(sys.argv[1] if len(sys.argv) > 1 else None)
    except Exception as why:
        sys.stdout.write("PROBLEM " + b64(str(why)) + "\n")
        sys.stdout.flush()
        sys.exit(3)
    bridge.run(commands)
    os._exit(0)


main()
