'use strict';
// A comprehensive VS Code extension host shim. It loads extension scripts with a
// complete `vscode` module shim and speaks newline-delimited JSON-RPC to Coiman over stdio.
// Nothing here runs until Coiman starts the host after the first frame.

const fs = require('fs');
const path = require('path');
const Module = require('module');

// Save the raw stdout write function so extension console.log calls never corrupt JSON-RPC.
const rawStdoutWrite = process.stdout.write.bind(process.stdout);

let nextId = 1;
const waiting = new Map();
const subscriptions = [];
const extensions = new Map();
const config = new Map();
const contextKeys = new Map();
const documents = new Map();
const outputChannels = new Map();
const globalState = new Map();
const workspaceState = new Map();
const providers = new Map();
const commands = new Map();
let workspaceFolders = [];
let activeEditor = null;
let visibleEditors = [];
let webviewPort = process.env.WEBVIEW_PORT ? Number(process.env.WEBVIEW_PORT) : 0;
const webviewPanels = new Map();
let log = (text) => send('log', { text: String(text) });

function send(method, params) {
  try {
    rawStdoutWrite(JSON.stringify({ jsonrpc: '2.0', method, params: params || {} }) + '\n');
  } catch (e) {
    // ignore serialize error
  }
}

function request(method, params) {
  return new Promise((resolve) => {
    const id = nextId++;
    waiting.set(id, resolve);
    send('request', { id, method, params: params || {} });
  });
}

function answer(id, result, error) {
  try {
    rawStdoutWrite(JSON.stringify({ jsonrpc: '2.0', id, result: result !== undefined ? result : null, error: error ? String(error) : null }) + '\n');
  } catch (e) {
    rawStdoutWrite(JSON.stringify({ jsonrpc: '2.0', id, result: null, error: String(e) }) + '\n');
  }
}

// Protect stdio JSON-RPC framing from arbitrary extension console output
console.log = (...args) => log(args.map((a) => (typeof a === 'object' ? JSON.stringify(a) : String(a))).join(' '));
console.info = (...args) => log('[info] ' + args.map((a) => (typeof a === 'object' ? JSON.stringify(a) : String(a))).join(' '));
console.warn = (...args) => log('[warn] ' + args.map((a) => (typeof a === 'object' ? JSON.stringify(a) : String(a))).join(' '));
console.error = (...args) => log('[error] ' + args.map((a) => (typeof a === 'object' ? JSON.stringify(a) : String(a))).join(' '));
console.debug = (...args) => log('[debug] ' + args.map((a) => (typeof a === 'object' ? JSON.stringify(a) : String(a))).join(' '));

process.on('uncaughtException', (err) => {
  log('uncaughtException: ' + (err && err.stack ? err.stack : String(err)));
});
process.on('unhandledRejection', (reason) => {
  log('unhandledRejection: ' + (reason && reason.stack ? reason.stack : String(reason)));
});

// ---------------------------------------------------------------- basics ----
class Disposable {
  constructor(fn) { this._fn = fn || (() => {}); this._disposed = false; }
  dispose() {
    if (!this._disposed) {
      this._disposed = true;
      try { this._fn(); } catch (e) { log('disposable failed: ' + e); }
    }
  }
  static from(...items) {
    const flat = [];
    for (const item of items) {
      if (Array.isArray(item)) flat.push(...item);
      else if (item) flat.push(item);
    }
    return new Disposable(() => {
      for (const i of flat) {
        if (i && typeof i.dispose === 'function') {
          try { i.dispose(); } catch (e) { log('disposable failed: ' + e); }
        }
      }
    });
  }
}

class ThemeColor { constructor(id) { this.id = id; } }

class ThemeIcon {
  constructor(id, color) { this.id = id; this.color = color; }
  static File = new ThemeIcon('file');
  static Folder = new ThemeIcon('folder');
}

class ColorTheme {
  constructor(kind = 1) { this.kind = kind; }
}

class EventEmitter {
  constructor() { this._listeners = []; }
  get event() {
    return (listener, thisArg, disposables) => {
      const bound = thisArg ? listener.bind(thisArg) : listener;
      this._listeners.push(bound);
      const d = new Disposable(() => {
        this._listeners = this._listeners.filter((l) => l !== bound);
      });
      if (disposables && Array.isArray(disposables)) disposables.push(d);
      return d;
    };
  }
  fire(value) {
    for (const l of this._listeners.slice()) {
      try { l(value); } catch (e) { log('listener failed: ' + e); }
    }
  }
  dispose() { this._listeners.length = 0; }
}

class CancellationTokenSource {
  constructor() {
    this._emitter = new EventEmitter();
    this.token = {
      isCancellationRequested: false,
      onCancellationRequested: this._emitter.event,
    };
  }
  cancel() {
    if (!this.token.isCancellationRequested) {
      this.token.isCancellationRequested = true;
      this._emitter.fire(undefined);
    }
  }
  dispose() { this._emitter.dispose(); }
}
const CancellationToken = {
  None: { isCancellationRequested: false, onCancellationRequested: new EventEmitter().event },
};
class CancellationError extends Error {
  constructor(message = 'Cancelled') { super(message); this.name = 'CancellationError'; }
}

class Position {
  constructor(line, character) { this.line = Math.max(0, line | 0); this.character = Math.max(0, character | 0); }
  isBefore(o) { return this.line < o.line || (this.line === o.line && this.character < o.character); }
  isAfter(o) { return o.isBefore(this); }
  isBeforeOrEqual(o) { return !this.isAfter(o); }
  isAfterOrEqual(o) { return !this.isBefore(o); }
  isEqual(o) { return this.line === o.line && this.character === o.character; }
  translate(l = 0, c = 0) { return new Position(this.line + l, this.character + c); }
  with(line, character) { return new Position(line === undefined ? this.line : line, character === undefined ? this.character : character); }
  compareTo(o) { return this.line - o.line || this.character - o.character; }
}

class Range {
  constructor(a, b, c, d) {
    if (a instanceof Position) {
      this.start = a;
      this.end = b instanceof Position ? b : a;
    } else {
      this.start = new Position(a || 0, b || 0);
      this.end = new Position(c !== undefined ? c : a || 0, d !== undefined ? d : b || 0);
    }
    if (this.end.isBefore(this.start)) {
      const tmp = this.start; this.start = this.end; this.end = tmp;
    }
  }
  get isEmpty() { return this.start.isEqual(this.end); }
  get isSingleLine() { return this.start.line === this.end.line; }
  contains(p) {
    if (p instanceof Range) return this.contains(p.start) && this.contains(p.end);
    return this.start.isBeforeOrEqual(p) && this.end.isAfterOrEqual(p);
  }
  isEqual(o) { return this.start.isEqual(o.start) && this.end.isEqual(o.end); }
  intersection(o) {
    const start = this.start.isAfter(o.start) ? this.start : o.start;
    const end = this.end.isBefore(o.end) ? this.end : o.end;
    return start.isBeforeOrEqual(end) ? new Range(start, end) : undefined;
  }
  union(o) {
    const start = this.start.isBefore(o.start) ? this.start : o.start;
    const end = this.end.isAfter(o.end) ? this.end : o.end;
    return new Range(start, end);
  }
  with(start, end) { return new Range(start || this.start, end || this.end); }
}

class Selection extends Range {
  constructor(anchor, active, b, c) {
    if (anchor instanceof Position) {
      super(anchor, active);
      this.anchor = anchor;
      this.active = active || anchor;
    } else {
      super(anchor, active, b, c);
      this.anchor = this.start;
      this.active = this.end;
    }
  }
  get isReversed() { return this.anchor.isAfter(this.active); }
}

class Location {
  constructor(uri, rangeOrPos) {
    this.uri = uri;
    this.range = rangeOrPos instanceof Range ? rangeOrPos : new Range(rangeOrPos, rangeOrPos);
  }
}

class LocationLink {
  constructor(targetUri, targetRange, targetSelectionRange, originSelectionRange) {
    this.targetUri = targetUri;
    this.targetRange = targetRange;
    this.targetSelectionRange = targetSelectionRange || targetRange;
    this.originSelectionRange = originSelectionRange;
  }
}

class Uri {
  constructor(scheme, authority, p, query, fragment) {
    this.scheme = scheme || 'file';
    this.authority = authority || '';
    this.path = p || '';
    this.query = query || '';
    this.fragment = fragment || '';
  }
  static file(p) {
    let normalized = (p || '').replace(/\\/g, '/');
    if (!normalized.startsWith('/') && /^[a-zA-Z]:/.test(normalized)) {
      normalized = '/' + normalized;
    }
    return new Uri('file', '', normalized);
  }
  static parse(value) {
    const m = /^([A-Za-z][\w+.-]*):\/\/([^/?#]*)([^?#]*)(\?[^#]*)?(#.*)?$/.exec(value || '');
    if (!m) return Uri.file(value || '');
    return new Uri(m[1], m[2], m[3], m[4] ? m[4].slice(1) : '', m[5] ? m[5].slice(1) : '');
  }
  static from(components) {
    return new Uri(components.scheme, components.authority, components.path, components.query, components.fragment);
  }
  static joinPath(uri, ...parts) {
    return new Uri(uri.scheme, uri.authority, path.posix.join(uri.path, ...parts), uri.query, uri.fragment);
  }
  get fsPath() {
    let p = decodeURIComponent(this.path);
    if (process.platform === 'win32' && /^\/[a-zA-Z]:/.test(p)) {
      p = p.slice(1).replace(/\//g, '\\');
    }
    return p;
  }
  toString() {
    return `${this.scheme}://${this.authority}${this.path}${this.query ? '?' + this.query : ''}${this.fragment ? '#' + this.fragment : ''}`;
  }
  toJSON() { return this.toString(); }
  asWebviewUri() { return Promise.resolve(this); }
  with(change) {
    return new Uri(
      change.scheme !== undefined ? change.scheme : this.scheme,
      change.authority !== undefined ? change.authority : this.authority,
      change.path !== undefined ? change.path : this.path,
      change.query !== undefined ? change.query : this.query,
      change.fragment !== undefined ? change.fragment : this.fragment
    );
  }
}

class MarkdownString {
  constructor(value = '', supportThemeIcons = false) {
    this.value = value || '';
    this.isTrusted = false;
    this.supportThemeIcons = supportThemeIcons;
    this.supportHtml = false;
    this.baseUri = undefined;
  }
  appendText(v) { this.value += (v || ''); return this; }
  appendMarkdown(v) { this.value += (v || ''); return this; }
  appendCodeblock(v, lang = '') { this.value += '\n```' + lang + '\n' + (v || '') + '\n```\n'; return this; }
}

class SnippetString {
  constructor(value = '') { this.value = value; }
  appendText(v) { this.value += v; return this; }
  appendTabstop(num = 0) { this.value += '$' + num; return this; }
  appendPlaceholder(v, num) { this.value += '${' + (num !== undefined ? num + ':' : '') + v + '}'; return this; }
  appendChoice(values, num = 1) { this.value += '${' + num + '|' + values.join(',') + '|}'; return this; }
  appendVariable(name, def = '') { this.value += '${' + name + (def ? ':' + def : '') + '}'; return this; }
}

class RelativePattern {
  constructor(base, pattern) {
    this.base = typeof base === 'string' ? base : (base.fsPath || base.path);
    this.baseUri = typeof base === 'string' ? Uri.file(base) : base;
    this.pattern = pattern;
  }
}

// ------------------------------------------------------------- enums & types -
const DiagnosticSeverity = { Error: 0, Warning: 1, Information: 2, Hint: 3 };
const DiagnosticTag = { Unnecessary: 1, Deprecated: 2 };
const CompletionItemKind = {
  Text: 0, Method: 1, Function: 2, Constructor: 3, Field: 4, Variable: 5, Class: 6, Struct: 7,
  Interface: 8, Module: 9, Property: 10, Event: 11, Operator: 12, Unit: 13, Value: 14, Constant: 15,
  Enum: 16, EnumMember: 17, Keyword: 18, Snippet: 19, Color: 20, File: 21, Reference: 22, Folder: 23,
  TypeParameter: 24, User: 25, Issue: 26,
};
const CompletionItemTag = { Deprecated: 1 };
const CompletionTriggerKind = { Invoke: 0, TriggerCharacter: 1, TriggerForIncompleteCompletions: 2 };
const CompletionItemInsertTextRule = { KeepWhitespace: 1, InsertAsSnippet: 4 };
const SymbolKind = {
  File: 0, Module: 1, Namespace: 2, Package: 3, Class: 4, Method: 5, Property: 6, Field: 7,
  Constructor: 8, Enum: 9, Interface: 10, Function: 11, Variable: 12, Constant: 13, String: 14,
  Number: 15, Boolean: 16, Array: 17, Object: 18, Key: 19, Null: 20, EnumMember: 21, Struct: 22,
  Event: 23, Operator: 24, TypeParameter: 25,
};
const SymbolTag = { Deprecated: 1 };
const CodeActionTriggerKind = { Invoke: 1, Automatic: 2 };
class CodeActionKind {
  constructor(val) { this.value = val; }
  static Empty = new CodeActionKind('');
  static QuickFix = new CodeActionKind('quickfix');
  static Refactor = new CodeActionKind('refactor');
  static RefactorExtract = new CodeActionKind('refactor.extract');
  static RefactorInline = new CodeActionKind('refactor.inline');
  static RefactorRewrite = new CodeActionKind('refactor.rewrite');
  static Source = new CodeActionKind('source');
  static SourceOrganizeImports = new CodeActionKind('source.organizeImports');
  static SourceFixAll = new CodeActionKind('source.fixAll');
  contains(o) {
    const val = typeof o === 'string' ? o : (o && o.value) || '';
    return val.startsWith(this.value);
  }
  intersects(o) {
    const val = typeof o === 'string' ? o : (o && o.value) || '';
    return this.contains(o) || val.startsWith(this.value);
  }
}
const ViewColumn = { Active: -1, Beside: -2, One: 1, Two: 2, Three: 3, Four: 4, Five: 5, Six: 6, Seven: 7, Eight: 8, Nine: 9 };
const StatusBarAlignment = { Left: 1, Right: 2 };
const ProgressLocation = { SourceControl: 1, Window: 10, Notification: 15 };
const ConfigurationTarget = { Global: 1, Workspace: 2, WorkspaceFolder: 3 };
const ExtensionMode = { Production: 1, Development: 2, Test: 3 };
const FileType = { Unknown: 0, File: 1, Directory: 2, SymbolicLink: 64 };
const FileChangeType = { Changed: 1, Created: 2, Deleted: 3 };
const TextDocumentSaveReason = { Manual: 1, AfterDelay: 2, FocusOut: 3 };
const EndOfLine = { LF: 1, CRLF: 2 };
const UIKind = { Desktop: 1, Web: 2 };
const ColorThemeKind = { Dark: 1, Light: 2, HighContrast: 3, HighContrastLight: 4 };
const LogLevel = { Off: 0, Trace: 1, Debug: 2, Info: 3, Warning: 4, Error: 5 };
const InlayHintKind = { Type: 1, Parameter: 2 };
const FoldingRangeKind = { Comment: 1, Imports: 2, Region: 3 };
const LanguageStatusSeverity = { Information: 0, Warning: 1, Error: 2 };
const TreeItemCollapsibleState = { None: 0, Collapsed: 1, Expanded: 2 };
const CommentThreadCollapsibleState = { Collapsed: 0, Expanded: 1 };
const CommentMode = { Editing: 0, Preview: 1 };
const DecorationRangeBehavior = { ClosedClosed: 0, ClosedOpen: 1, OpenClosed: 2, OpenOpen: 3 };
const ShellQuoting = { Escape: 1, Strong: 2, Weak: 3 };
const TaskScope = { Global: 1, Workspace: 2 };
const TaskRevealKind = { Always: 1, Silent: 2, Never: 3 };
const TaskPanelKind = { Shared: 1, Dedicated: 2, New: 3 };

class ShellExecution {
  constructor(commandLineOrCommand, argsOrOptions, options) {
    if (Array.isArray(argsOrOptions)) {
      this.command = commandLineOrCommand;
      this.args = argsOrOptions;
      this.options = options || {};
    } else {
      this.commandLine = commandLineOrCommand;
      this.options = argsOrOptions || {};
    }
  }
}

class ProcessExecution {
  constructor(process, args, options) {
    this.process = process;
    this.args = args || [];
    this.options = options || {};
  }
}

class CustomExecution {
  constructor(callback) {
    this.callback = callback;
  }
}

class TaskGroup {
  constructor(id, label) { this.id = id; this.label = label; }
  static Clean = new TaskGroup('clean', 'Clean');
  static Build = new TaskGroup('build', 'Build');
  static Rebuild = new TaskGroup('rebuild', 'Rebuild');
  static Test = new TaskGroup('test', 'Test');
}

class Task {
  constructor(definition, scopeOrTarget, name, source, execution, problemMatchers) {
    this.definition = definition;
    this.scope = scopeOrTarget;
    this.name = name;
    this.source = source;
    this.execution = execution;
    this.problemMatchers = Array.isArray(problemMatchers) ? problemMatchers : (problemMatchers ? [problemMatchers] : []);
    this.isBackground = false;
    this.presentationOptions = {};
    this.runOptions = {};
  }
}

let nextTerminalId = 1;
const terminalInstances = new Map();
let activeTerminalInstance = undefined;
const onDidChangeActiveTerminal = new EventEmitter();
const onDidOpenTerminal = new EventEmitter();
const onDidCloseTerminal = new EventEmitter();

class Terminal {
  constructor(id, name, options) {
    this._id = id;
    this.name = name || 'Terminal';
    this.creationOptions = options || {};
    this.processId = Promise.resolve(undefined);
    this.exitStatus = undefined;
    this.state = { isInteractedWith: true };
    this._disposed = false;
  }
  sendText(text, addNewLine = true) {
    if (this._disposed) return;
    send('terminal/sendText', { id: this._id, text: String(text), addNewLine: addNewLine !== false });
  }
  show(preserveFocus = false) {
    if (this._disposed) return;
    activeTerminalInstance = this;
    onDidChangeActiveTerminal.fire(this);
    send('terminal/show', { id: this._id, preserveFocus: !!preserveFocus });
  }
  hide() {
    if (this._disposed) return;
    send('terminal/hide', { id: this._id });
  }
  dispose() {
    if (this._disposed) return;
    this._disposed = true;
    terminalInstances.delete(this._id);
    if (activeTerminalInstance === this) {
      activeTerminalInstance = Array.from(terminalInstances.values())[0];
      onDidChangeActiveTerminal.fire(activeTerminalInstance);
    }
    onDidCloseTerminal.fire(this);
    send('terminal/dispose', { id: this._id });
  }
}

const activeWatchers = new Set();
function notifyWatchers(uri, type) {
  for (const watcher of activeWatchers) {
    try {
      if (type === FileChangeType.Created && watcher._created) watcher._created.fire(uri);
      else if (type === FileChangeType.Changed && watcher._changed) watcher._changed.fire(uri);
      else if (type === FileChangeType.Deleted && watcher._deleted) watcher._deleted.fire(uri);
    } catch (e) {
      log('watcher notification failed: ' + e);
    }
  }
}

// ------------------------------------------------------------ documents ----
class TextDocument {
  constructor(init) {
    this.uri = init.uri instanceof Uri ? init.uri : Uri.parse(init.uri || '');
    this.languageId = init.languageId || 'plaintext';
    this.version = init.version || 1;
    this._text = init.text || '';
    this.fileName = this.uri.fsPath;
    this.isUntitled = this.uri.scheme === 'untitled';
    this.isClosed = false;
    this.isDirty = false;
    this.eol = (init.eol === 2 ? EndOfLine.CRLF : EndOfLine.LF);
    this._onDidChange = new EventEmitter();
    this._onDidSave = new EventEmitter();
  }
  get text() { return this._text; }
  get lineCount() { return this._text.split(/\r?\n/).length; }
  getText(range) {
    if (!range) return this._text;
    const start = this.offsetAt(range.start), end = this.offsetAt(range.end);
    return this._text.slice(start, end);
  }
  offsetAt(pos) {
    const lines = this._text.split(/\r?\n/);
    let offset = 0;
    for (let i = 0; i < pos.line && i < lines.length; i++) offset += lines[i].length + 1;
    return offset + Math.min(pos.character, (lines[pos.line] || '').length);
  }
  positionAt(offset) {
    const before = this._text.slice(0, Math.max(0, offset)).split(/\r?\n/);
    return new Position(before.length - 1, before[before.length - 1].length);
  }
  lineAt(lineOrPos) {
    const n = typeof lineOrPos === 'number' ? lineOrPos : lineOrPos.line;
    const lines = this._text.split(/\r?\n/);
    const text = lines[n] || '';
    const firstNonWs = text.search(/\S/);
    return {
      lineNumber: n,
      text,
      range: new Range(n, 0, n, text.length),
      rangeIncludingLineBreak: new Range(n, 0, n + 1, 0),
      firstNonWhitespaceCharacterIndex: firstNonWs === -1 ? text.length : firstNonWs,
      isEmptyOrWhitespace: /^\s*$/.test(text),
    };
  }
  getWordRangeAtPosition(position, regex) {
    const line = this.lineAt(position.line);
    const text = line.text;
    const re = regex || /(-?\d*\.\d\w*)|([^\`\~\!\@\#\$\%\^\&\*\(\)\-\=\+\[\{\]\}\\\|\;\:\'\"\,\.\<\>\/\?\s]+)/g;
    let match;
    while ((match = re.exec(text)) !== null) {
      const start = match.index;
      const end = start + match[0].length;
      if (position.character >= start && position.character <= end) {
        return new Range(position.line, start, position.line, end);
      }
    }
    return undefined;
  }
  validateRange(range) {
    const start = this.validatePosition(range.start);
    const end = this.validatePosition(range.end);
    return new Range(start, end);
  }
  validatePosition(pos) {
    const lines = this._text.split(/\r?\n/);
    const line = Math.max(0, Math.min(pos.line, lines.length - 1));
    const character = Math.max(0, Math.min(pos.character, (lines[line] || '').length));
    return new Position(line, character);
  }
  save() { return Promise.resolve(true); }
  _update(text, version) {
    this._text = text;
    this.version = version;
    const changeEvent = { document: this, contentChanges: [{ text }], reason: undefined };
    this._onDidChange.fire(changeEvent);
    onDidChangeTextDocument.fire(changeEvent);
  }
}

class TextEditor {
  constructor(document) {
    this.document = document;
    this.selection = new Selection(new Position(0, 0), new Position(0, 0));
    this.selections = [this.selection];
    this.visibleRanges = [new Range(new Position(0, 0), new Position(Math.min(50, document.lineCount), 0))];
    this.viewColumn = ViewColumn.One;
    this.options = { tabSize: 4, insertSpaces: true };
  }
  edit(callback, options) {
    const edits = [];
    const builder = {
      replace: (range, text) => edits.push({ range, newText: String(text) }),
      insert: (position, text) => edits.push({ range: new Range(position, position), newText: String(text) }),
      delete: (range) => edits.push({ range, newText: '' }),
      setEndOfLine: (eol) => {},
    };
    try {
      const res = callback(builder);
      if (edits.length > 0) {
        const we = new WorkspaceEdit();
        for (const e of edits) we.replace(this.document.uri, e.range, e.newText);
        return vscode.workspace.applyEdit(we).then(() => true);
      }
      return Promise.resolve(true);
    } catch (e) {
      return Promise.reject(e);
    }
  }
  insertSnippet(snippet, location, options) {
    const text = snippet instanceof SnippetString ? snippet.value.replace(/\$\{\d+:?([^}]*)\}/g, '$1').replace(/\$\d+/g, '') : String(snippet);
    const pos = location instanceof Position ? location : (location instanceof Range ? location.start : this.selection.active);
    return this.edit((b) => b.insert(pos, text));
  }
  revealRange(range, revealType) {
    send('editor/reveal', { uri: this.document.uri.toString(), range });
  }
  setDecorations(decorationType, rangesOrOptions) {
    const key = decorationType && decorationType.key ? decorationType.key : String(decorationType);
    const ranges = Array.isArray(rangesOrOptions) ? rangesOrOptions.map((r) => {
      if (r instanceof Range) {
        return { range: r };
      } else if (r && r.range) {
        return { range: r.range, hoverMessage: r.hoverMessage, renderOptions: r.renderOptions };
      }
      return { range: r };
    }) : [];
    send('window/setDecorations', { key, uri: this.document.uri.toString(), ranges });
  }
}

class TextEdit {
  constructor(range, newText) { this.range = range; this.newText = newText || ''; }
  static replace(range, newText) { return new TextEdit(range, newText); }
  static insert(position, newText) { return new TextEdit(new Range(position, position), newText); }
  static delete(range) { return new TextEdit(range, ''); }
  static setEndOfLine(eol) { return { newEol: eol }; }
}

class SnippetTextEdit {
  constructor(range, snippet) { this.range = range; this.snippet = snippet; }
}

class WorkspaceEdit {
  constructor() { this._edits = []; }
  insert(uri, position, newText) { this._edits.push([uri.toString(), { range: new Range(position, position), newText }]); }
  delete(uri, range) { this._edits.push([uri.toString(), { range, newText: '' }]); }
  replace(uri, range, newText) { this._edits.push([uri.toString(), { range, newText }]); }
  createFile(uri, options) { this._edits.push([uri.toString(), { createFile: true, options }]); }
  deleteFile(uri, options) { this._edits.push([uri.toString(), { deleteFile: true, options }]); }
  renameFile(oldUri, newUri, options) { this._edits.push([oldUri.toString(), { renameFile: newUri.toString(), options }]); }
  set(uri, edits) {
    const key = uri.toString();
    for (const e of edits || []) {
      if (e instanceof TextEdit) this._edits.push([key, e]);
      else if (e && e.range) this._edits.push([key, { range: e.range, newText: e.newText || '' }]);
    }
  }
  get(uri) {
    const key = uri.toString();
    return this._edits.filter(([u]) => u === key).map(([, e]) => e);
  }
  has(uri) { return this.get(uri).length > 0; }
  entries() { return this._edits; }
  get size() { return this._edits.length; }
}

class DiagnosticCollection {
  constructor(name) {
    this.name = name || 'extension';
    this._map = new Map();
    this._emitter = new EventEmitter();
  }
  get onDidChangeDiagnostics() { return this._emitter.event; }
  set(uri, diagnostics) {
    if (Array.isArray(uri)) {
      for (const [u, d] of uri) this.set(u, d);
      return;
    }
    const key = typeof uri === 'string' ? uri : uri.toString();
    this._map.set(key, diagnostics || []);
    this._emitter.fire([uri]);
    send('diagnostics', { uri: key, diagnostics: diagnostics || [], name: this.name });
  }
  delete(uri) {
    const key = typeof uri === 'string' ? uri : uri.toString();
    this._map.delete(key);
    send('diagnostics', { uri: key, diagnostics: [], name: this.name });
  }
  clear() {
    for (const key of this._map.keys()) {
      send('diagnostics', { uri: key, diagnostics: [], name: this.name });
    }
    this._map.clear();
  }
  get(uri) { return this._map.get(typeof uri === 'string' ? uri : uri.toString()) || []; }
  get entries() { return Array.from(this._map.entries()).map(([u, d]) => [Uri.parse(u), d]); }
  dispose() { this.clear(); }
}

// -------------------------------------------------------------- selectors ----
function matches(selector, document) {
  if (selector === undefined || selector === null || selector === '' || selector === '*') return true;
  const list = Array.isArray(selector) ? selector : [selector];
  return list.some((s) => {
    if (typeof s === 'string') return s === '*' || document.languageId === s;
    if (s.language !== undefined && s.language !== '*' && s.language !== document.languageId) return false;
    if (s.scheme !== undefined && s.scheme !== document.uri.scheme) return false;
    if (s.pattern) {
      const p = typeof s.pattern === 'string' ? s.pattern : s.pattern.pattern;
      if (p && !document.fileName.endsWith(p.replace(/^\*\*\//, ''))) return false;
    }
    return true;
  });
}

function providerFor(kind, document) {
  const list = providers.get(kind) || [];
  return list.find((p) => matches(p.selector, document));
}

function configurationFor(section) {
  const prefix = section ? section + '.' : '';
  const target = {
    get(key, fallback) {
      if (!key) return this;
      const full = prefix + key;
      if (config.has(full)) return config.get(full);
      // Try nested lookup if key has dots or prefix matches
      for (const [k, v] of config.entries()) {
        if (k === full) return v;
        if (k.startsWith(full + '.')) return v;
      }
      return fallback;
    },
    has(key) { return config.has(prefix + key); },
    inspect(key) {
      const full = prefix + key;
      return { key: full, defaultValue: undefined, globalValue: config.get(full), workspaceValue: config.get(full) };
    },
    update(key, value) {
      const full = prefix + key;
      if (value === undefined) config.delete(full);
      else config.set(full, value);
      send('configuration/update', { section: full, value });
      return Promise.resolve();
    },
  };
  return new Proxy(target, {
    get(obj, prop, receiver) {
      if (typeof prop === 'string') {
        if (prop in obj) return obj[prop];
        return obj.get(prop);
      }
      return Reflect.get(obj, prop, receiver);
    },
    has(obj, prop) {
      if (typeof prop === 'string') {
        if (prop in obj) return true;
        return obj.has(prop);
      }
      return Reflect.has(obj, prop);
    },
  });
}

function outputChannel(name) {
  const emitter = new EventEmitter();
  return {
    name,
    append(v) { send('output', { name, text: String(v) }); },
    appendLine(v) { send('output', { name, text: String(v) + '\n' }); },
    info(v) { send('output', { name, text: '[info] ' + String(v) + '\n' }); },
    warn(v) { send('output', { name, text: '[warn] ' + String(v) + '\n' }); },
    error(v) { send('output', { name, text: '[error] ' + String(v) + '\n' }); },
    debug(v) { send('output', { name, text: '[debug] ' + String(v) + '\n' }); },
    trace(v) { send('output', { name, text: '[trace] ' + String(v) + '\n' }); },
    clear() {},
    show() {},
    hide() {},
    replace(v) { send('output', { name, text: String(v), replace: true }); },
    dispose() { outputChannels.delete(name); emitter.fire(); },
    onDidDispose: emitter.event,
  };
}

function memento(map) {
  return {
    keys() { return Array.from(map.keys()); },
    get(key, fallback) { return map.has(key) ? map.get(key) : fallback; },
    update(key, value) {
      if (value === undefined) map.delete(key);
      else map.set(key, value);
      return Promise.resolve();
    },
    setKeysForSync(keys) { map.set('__syncKeys', keys); return Promise.resolve(); },
  };
}

function extensionContext(extension) {
  const dir = extension.dir;
  return {
    subscriptions,
    extensionPath: dir,
    extensionUri: Uri.file(dir),
    globalState: memento(globalState),
    workspaceState: memento(workspaceState),
    secrets: {
      get: async () => undefined,
      store: async () => {},
      delete: async () => {},
      onDidChange: new EventEmitter().event,
    },
    storageUri: Uri.file(path.join(dir, '.storage')),
    globalStorageUri: Uri.file(path.join(dir, '.global')),
    logUri: Uri.file(path.join(dir, '.log')),
    extensionMode: ExtensionMode.Production,
    environmentVariableCollection: {
      persistent: false,
      replace() {}, append() {}, prepend() {}, clear() {},
      get() { return undefined; },
    },
    asAbsolutePath(p) { return path.join(dir, p); },
    extension,
  };
}

// ------------------------------------------------------------------ vscode ----
const onDidChangeTextDocument = new EventEmitter();
const onDidOpenTextDocument = new EventEmitter();
const onDidCloseTextDocument = new EventEmitter();
const onDidSaveTextDocument = new EventEmitter();
const onWillSaveTextDocument = new EventEmitter();
const onDidChangeActiveTextEditor = new EventEmitter();
const onDidChangeTextEditorSelection = new EventEmitter();
const onDidChangeTextEditorVisibleRanges = new EventEmitter();
const onDidChangeVisibleTextEditors = new EventEmitter();
const onDidChangeWindowState = new EventEmitter();
const onDidChangeActiveColorTheme = new EventEmitter();
const onDidChangeConfiguration = new EventEmitter();
const onDidChangeWorkspaceFolders = new EventEmitter();
const onDidCreateFiles = new EventEmitter();
const onDidDeleteFiles = new EventEmitter();
const onDidRenameFiles = new EventEmitter();

let nextDecorationId = 1;
let nextStatusBarId = 1;
let nextWebviewId = 1;
const taskProviders = new Map();
const onDidStartTask = new EventEmitter();
const onDidEndTask = new EventEmitter();
const onDidStartTaskProcess = new EventEmitter();
const onDidEndTaskProcess = new EventEmitter();

const vscode = {
  version: '1.99.0',
  Disposable, EventEmitter, CancellationTokenSource, CancellationToken, CancellationError,
  ThemeColor, ThemeIcon, ColorTheme, ColorThemeKind,
  Position, Range, Selection, Location, LocationLink, Uri, MarkdownString, SnippetString,
  RelativePattern, DiagnosticSeverity, DiagnosticTag, CompletionItemKind, CompletionItemTag,
  CompletionTriggerKind, CompletionItemInsertTextRule, SymbolKind, SymbolTag, CodeActionKind,
  CodeActionTriggerKind, ViewColumn, StatusBarAlignment, ProgressLocation, ConfigurationTarget,
  ExtensionMode, FileType, FileChangeType, TextDocumentSaveReason, EndOfLine, UIKind, LogLevel, InlayHintKind,
  FoldingRangeKind, LanguageStatusSeverity, TreeItemCollapsibleState, CommentThreadCollapsibleState,
  CommentMode, DecorationRangeBehavior, ShellQuoting, TaskScope, TaskRevealKind, TaskPanelKind,
  ShellExecution, ProcessExecution, CustomExecution, TaskGroup, Task, Terminal,

  Diagnostic: class Diagnostic {
    constructor(range, message, severity = DiagnosticSeverity.Error) {
      this.range = range;
      this.message = message;
      this.severity = severity;
      this.source = '';
      this.code = undefined;
      this.relatedInformation = [];
      this.tags = [];
    }
  },
  DiagnosticRelatedInformation: class DiagnosticRelatedInformation {
    constructor(location, message) { this.location = location; this.message = message; }
  },
  CompletionItem: class CompletionItem {
    constructor(label, kind) {
      this.label = label;
      this.kind = kind;
      this.detail = '';
      this.documentation = undefined;
      this.sortText = undefined;
      this.filterText = undefined;
      this.preselect = false;
      this.insertText = undefined;
      this.range = undefined;
      this.commitCharacters = undefined;
      this.additionalTextEdits = undefined;
      this.command = undefined;
      this.tags = [];
    }
  },
  CompletionList: class CompletionList {
    constructor(items = [], isIncomplete = false) {
      this.items = items;
      this.isIncomplete = isIncomplete;
    }
  },
  Hover: class Hover {
    constructor(contents, range) {
      this.contents = Array.isArray(contents) ? contents : [contents];
      this.range = range;
    }
  },
  CodeAction: class CodeAction {
    constructor(title, kind) {
      this.title = title;
      this.kind = kind;
      this.diagnostics = [];
      this.isPreferred = false;
      this.edit = undefined;
      this.command = undefined;
      this.disabled = undefined;
    }
  },
  Command: class Command {
    constructor(title, command, ...args) {
      this.title = title;
      this.command = command;
      this.arguments = args;
    }
  },
  CodeLens: class CodeLens {
    constructor(range, command) { this.range = range; this.command = command; this.isResolved = !!command; }
  },
  DocumentLink: class DocumentLink {
    constructor(range, target) { this.range = range; this.target = target; this.tooltip = undefined; }
  },
  SymbolInformation: class SymbolInformation {
    constructor(name, kind, range, uri, containerName) {
      Object.assign(this, { name, kind, location: new Location(uri, range), containerName });
    }
  },
  DocumentSymbol: class DocumentSymbol {
    constructor(name, detail, kind, range, selectionRange) {
      Object.assign(this, { name, detail, kind, range, selectionRange, children: [] });
    }
  },
  CallHierarchyItem: class CallHierarchyItem {
    constructor(kind, name, detail, uri, range, selectionRange) {
      Object.assign(this, { kind, name, detail, uri, range, selectionRange });
    }
  },
  CallHierarchyIncomingCall: class CallHierarchyIncomingCall {
    constructor(from, fromRanges) { this.from = from; this.fromRanges = fromRanges; }
  },
  CallHierarchyOutgoingCall: class CallHierarchyOutgoingCall {
    constructor(to, fromRanges) { this.to = to; this.fromRanges = fromRanges; }
  },
  TypeHierarchyItem: class TypeHierarchyItem {
    constructor(kind, name, detail, uri, range, selectionRange) {
      Object.assign(this, { kind, name, detail, uri, range, selectionRange });
    }
  },
  InlayHint: class InlayHint {
    constructor(position, label, kind) { Object.assign(this, { position, label, kind }); }
  },
  InlayHintLabelPart: class InlayHintLabelPart {
    constructor(value) { this.value = value; this.tooltip = undefined; this.location = undefined; this.command = undefined; }
  },
  SelectionRange: class SelectionRange {
    constructor(range, parent) { this.range = range; this.parent = parent; }
  },
  FoldingRange: class FoldingRange {
    constructor(start, end, kind) { Object.assign(this, { start, end, kind }); }
  },
  SemanticTokensLegend: class SemanticTokensLegend {
    constructor(tokenTypes = [], tokenModifiers = []) { this.tokenTypes = tokenTypes; this.tokenModifiers = tokenModifiers; }
  },
  SemanticTokensBuilder: class SemanticTokensBuilder {
    constructor(legend) { this._legend = legend; this._data = []; }
    push(line, char, length, tokenType, tokenModifiers = 0) {
      this._data.push(line, char, length, tokenType, tokenModifiers);
    }
    build(resultId) { return { data: new Uint32Array(this._data), resultId }; }
  },
  SemanticTokens: class SemanticTokens {
    constructor(data, resultId) { this.data = data; this.resultId = resultId; }
  },
  SignatureHelp: class SignatureHelp {
    constructor() { this.signatures = []; this.activeSignature = 0; this.activeParameter = 0; }
  },
  SignatureInformation: class SignatureInformation {
    constructor(label, documentation) { this.label = label; this.documentation = documentation; this.parameters = []; }
  },
  ParameterInformation: class ParameterInformation {
    constructor(label, documentation) { this.label = label; this.documentation = documentation; }
  },
  InlineValueText: class InlineValueText { constructor(range, text) { Object.assign(this, { range, text }); } },
  InlineValueVariableLookup: class InlineValueVariableLookup { constructor(range, variableName, caseSensitiveLookup) { Object.assign(this, { range, variableName, caseSensitiveLookup }); } },
  InlineValueEvaluatableExpression: class InlineValueEvaluatableExpression { constructor(range, expression) { Object.assign(this, { range, expression }); } },
  TextEdit,
  SnippetTextEdit,
  WorkspaceEdit,
  Task,
  TaskGroup,
  ShellExecution,
  ProcessExecution,
  CustomExecution,
  Terminal,
  TreeItem: class TreeItem {
    constructor(label, collapsibleState = TreeItemCollapsibleState.None) {
      this.label = label;
      this.collapsibleState = collapsibleState;
      this.id = undefined;
      this.iconPath = undefined;
      this.description = undefined;
      this.resourceUri = undefined;
      this.tooltip = undefined;
      this.command = undefined;
      this.contextValue = undefined;
      this.accessibilityInformation = undefined;
    }
  },
  StatusBarItem: class StatusBarItem {
    constructor(id, alignment = StatusBarAlignment.Left, priority = 0) {
      this.id = id || ('status_' + nextStatusBarId++);
      this.alignment = alignment;
      this.priority = priority;
      this.text = '';
      this.tooltip = '';
      this.command = '';
      this.color = undefined;
      this.backgroundColor = undefined;
      this.name = '';
      this._e = new EventEmitter();
    }
    get onDidChange() { return this._e.event; }
    show() {
      send('statusBar/update', {
        id: this.id, text: this.text, tooltip: this.tooltip,
        command: this.command, color: this.color, alignment: this.alignment, priority: this.priority,
      });
    }
    hide() { send('statusBar/hide', { id: this.id }); }
    dispose() { this.hide(); }
  },

  // ------------------------------------------------------------- workspace --
  workspace: {
    get workspaceFolders() { return workspaceFolders; },
    get name() { return workspaceFolders[0] ? workspaceFolders[0].name : undefined; },
    get workspaceFile() { return undefined; },
    get textDocuments() { return Array.from(documents.values()); },
    get isTrusted() { return true; },
    get onDidGrantWorkspaceTrust() { return new EventEmitter().event; },
    get onDidChangeTextDocument() { return onDidChangeTextDocument.event; },
    get onDidOpenTextDocument() { return onDidOpenTextDocument.event; },
    get onDidCloseTextDocument() { return onDidCloseTextDocument.event; },
    get onDidSaveTextDocument() { return onDidSaveTextDocument.event; },
    get onWillSaveTextDocument() { return onWillSaveTextDocument.event; },
    get onDidChangeConfiguration() { return onDidChangeConfiguration.event; },
    get onDidChangeWorkspaceFolders() { return onDidChangeWorkspaceFolders.event; },
    get onDidCreateFiles() { return onDidCreateFiles.event; },
    get onDidDeleteFiles() { return onDidDeleteFiles.event; },
    get onDidRenameFiles() { return onDidRenameFiles.event; },
    getConfiguration(section, scope) { return configurationFor(section); },
    getWorkspaceFolder(uri) {
      const fsPath = uri instanceof Uri ? uri.fsPath : String(uri);
      return workspaceFolders.find((f) => fsPath.startsWith(f.uri.fsPath));
    },
    asRelativePath(pathOrUri, includeWorkspaceFolder) {
      const p = typeof pathOrUri === 'string' ? pathOrUri : pathOrUri.fsPath;
      const root = workspaceFolders[0];
      return root && p.startsWith(root.uri.fsPath) ? path.relative(root.uri.fsPath, p) : p;
    },
    openTextDocument(arg) {
      if (typeof arg === 'string') {
        const u = Uri.file(arg);
        if (documents.has(u.toString())) return Promise.resolve(documents.get(u.toString()));
        const text = fs.existsSync(arg) ? fs.readFileSync(arg, 'utf8') : '';
        const doc = new TextDocument({ uri: u, languageId: 'plaintext', text });
        documents.set(u.toString(), doc);
        return Promise.resolve(doc);
      }
      if (arg && arg.uri) {
        const key = arg.uri.toString();
        if (documents.has(key)) return Promise.resolve(documents.get(key));
        const fsPath = arg.uri.fsPath;
        const text = fsPath && fs.existsSync(fsPath) ? fs.readFileSync(fsPath, 'utf8') : (arg.content || '');
        const doc = new TextDocument({ uri: arg.uri, languageId: arg.language || 'plaintext', text });
        documents.set(key, doc);
        return Promise.resolve(doc);
      }
      return Promise.resolve(new TextDocument({ uri: Uri.parse('untitled:untitled-1'), languageId: 'plaintext', text: '' }));
    },
    applyEdit(edit) {
      const entries = typeof edit.entries === 'function' ? edit.entries() : [];
      for (const [uri, change] of entries) {
        if (change.createFile) {
          const p = Uri.parse(uri).fsPath;
          if (!fs.existsSync(p)) {
            fs.mkdirSync(path.dirname(p), { recursive: true });
            fs.writeFileSync(p, '');
            notifyWatchers(Uri.parse(uri), FileChangeType.Created);
            onDidCreateFiles.fire({ files: [Uri.parse(uri)] });
            send('workspace/fileCreated', { uri });
          }
        } else if (change.deleteFile) {
          const p = Uri.parse(uri).fsPath;
          if (fs.existsSync(p)) {
            fs.rmSync(p, { recursive: true, force: true });
            notifyWatchers(Uri.parse(uri), FileChangeType.Deleted);
            onDidDeleteFiles.fire({ files: [Uri.parse(uri)] });
            send('workspace/fileDeleted', { uri });
          }
        } else if (change.renameFile) {
          const oldP = Uri.parse(uri).fsPath;
          const newP = Uri.parse(change.renameFile).fsPath;
          if (fs.existsSync(oldP)) {
            fs.renameSync(oldP, newP);
            notifyWatchers(Uri.parse(uri), FileChangeType.Deleted);
            notifyWatchers(Uri.parse(change.renameFile), FileChangeType.Created);
            onDidRenameFiles.fire({ files: [{ oldUri: Uri.parse(uri), newUri: Uri.parse(change.renameFile) }] });
            send('workspace/fileRenamed', { oldUri: uri, newUri: change.renameFile });
          }
        } else {
          const doc = documents.get(uri);
          if (doc && change.range && change.newText !== undefined) {
            const start = doc.offsetAt(change.range.start), end = doc.offsetAt(change.range.end);
            doc._update(doc.text.slice(0, start) + change.newText + doc.text.slice(end), doc.version + 1);
          }
        }
      }
      send('workspace/applyEdit', { edits: entries });
      return Promise.resolve(true);
    },
    createFileSystemWatcher(glob, ignoreCreate, ignoreChange, ignoreDelete) {
      const created = new EventEmitter(), changed = new EventEmitter(), deleted = new EventEmitter();
      const watcher = {
        _created: created,
        _changed: changed,
        _deleted: deleted,
        onDidCreate: created.event,
        onDidChange: changed.event,
        onDidDelete: deleted.event,
        dispose() { activeWatchers.delete(watcher); },
      };
      activeWatchers.add(watcher);
      return watcher;
    },
    findFiles(include, exclude, maxResults = 1000) {
      const results = [];
      const skipDirs = new Set(['.git', 'node_modules', 'target', 'dist', 'build', '.cache']);
      function scan(dir) {
        if (results.length >= maxResults) return;
        try {
          const entries = fs.readdirSync(dir, { withFileTypes: true });
          for (const e of entries) {
            if (e.isDirectory()) {
              if (!skipDirs.has(e.name) && !e.name.startsWith('.')) {
                scan(path.join(dir, e.name));
              }
            } else if (e.isFile()) {
              results.push(Uri.file(path.join(dir, e.name)));
              if (results.length >= maxResults) break;
            }
          }
        } catch (e) {}
      }
      for (const f of workspaceFolders) {
        if (fs.existsSync(f.uri.fsPath)) scan(f.uri.fsPath);
      }
      return Promise.resolve(results);
    },
    registerTextDocumentContentProvider(scheme, provider) {
      providers.set('content:' + scheme, provider);
      return new Disposable(() => providers.delete('content:' + scheme));
    },
    registerFileSystemProvider(scheme, provider, options) {
      providers.set('fs:' + scheme, provider);
      return new Disposable(() => providers.delete('fs:' + scheme));
    },
    saveAll(includeUntitled) { return Promise.resolve(true); },
    fs: {
      readFile(uri) { return Promise.resolve(Buffer.from(fs.readFileSync(uri.fsPath))); },
      writeFile(uri, content) {
        const u = uri instanceof Uri ? uri : Uri.parse(String(uri));
        const exists = fs.existsSync(u.fsPath);
        fs.writeFileSync(u.fsPath, content);
        notifyWatchers(u, exists ? FileChangeType.Changed : FileChangeType.Created);
        if (!exists) onDidCreateFiles.fire({ files: [u] });
        send(exists ? 'workspace/fileChanged' : 'workspace/fileCreated', { uri: u.toString() });
        return Promise.resolve();
      },
      stat(uri) {
        const s = fs.statSync(uri.fsPath);
        return Promise.resolve({
          type: s.isDirectory() ? FileType.Directory : FileType.File,
          size: s.size, mtime: s.mtimeMs, ctime: s.ctimeMs,
        });
      },
      readDirectory(uri) {
        return Promise.resolve(fs.readdirSync(uri.fsPath, { withFileTypes: true }).map((e) => [e.name, e.isDirectory() ? FileType.Directory : FileType.File]));
      },
      createDirectory(uri) {
        const u = uri instanceof Uri ? uri : Uri.parse(String(uri));
        fs.mkdirSync(u.fsPath, { recursive: true });
        notifyWatchers(u, FileChangeType.Created);
        onDidCreateFiles.fire({ files: [u] });
        send('workspace/fileCreated', { uri: u.toString() });
        return Promise.resolve();
      },
      delete(uri, options) {
        const u = uri instanceof Uri ? uri : Uri.parse(String(uri));
        fs.rmSync(u.fsPath, { recursive: (options && options.recursive) !== false, force: true });
        notifyWatchers(u, FileChangeType.Deleted);
        onDidDeleteFiles.fire({ files: [u] });
        send('workspace/fileDeleted', { uri: u.toString() });
        return Promise.resolve();
      },
      rename(a, b, options) {
        const uA = a instanceof Uri ? a : Uri.parse(String(a));
        const uB = b instanceof Uri ? b : Uri.parse(String(b));
        fs.renameSync(uA.fsPath, uB.fsPath);
        notifyWatchers(uA, FileChangeType.Deleted);
        notifyWatchers(uB, FileChangeType.Created);
        onDidRenameFiles.fire({ files: [{ oldUri: uA, newUri: uB }] });
        send('workspace/fileRenamed', { oldUri: uA.toString(), newUri: uB.toString() });
        return Promise.resolve();
      },
      copy(a, b, options) {
        const uA = a instanceof Uri ? a : Uri.parse(String(a));
        const uB = b instanceof Uri ? b : Uri.parse(String(b));
        fs.copyFileSync(uA.fsPath, uB.fsPath);
        notifyWatchers(uB, FileChangeType.Created);
        onDidCreateFiles.fire({ files: [uB] });
        send('workspace/fileCreated', { uri: uB.toString() });
        return Promise.resolve();
      },
    },
  },

  // ---------------------------------------------------------------- window --
  window: {
    get activeTextEditor() { return activeEditor; },
    get visibleTextEditors() { return visibleEditors; },
    get activeColorTheme() { return new ColorTheme(ColorThemeKind.Dark); },
    get state() { return { focused: true, active: true }; },
    get terminals() { return Array.from(terminalInstances.values()); },
    get activeTerminal() { return activeTerminalInstance; },
    get onDidOpenTerminal() { return onDidOpenTerminal.event; },
    get onDidCloseTerminal() { return onDidCloseTerminal.event; },
    get onDidChangeActiveTextEditor() { return onDidChangeActiveTextEditor.event; },
    get onDidChangeTextEditorSelection() { return onDidChangeTextEditorSelection.event; },
    get onDidChangeTextEditorVisibleRanges() { return onDidChangeTextEditorVisibleRanges.event; },
    get onDidChangeVisibleTextEditors() { return onDidChangeVisibleTextEditors.event; },
    get onDidChangeActiveTerminal() { return onDidChangeActiveTerminal.event; },
    get onDidChangeWindowState() { return onDidChangeWindowState.event; },
    get onDidChangeActiveColorTheme() { return onDidChangeActiveColorTheme.event; },

    showInformationMessage(m, ...items) {
      send('message', { level: 'info', text: String(m) });
      const first = items.find((i) => typeof i === 'string' || (i && i.title));
      return Promise.resolve(first);
    },
    showWarningMessage(m, ...items) {
      send('message', { level: 'warning', text: String(m) });
      const first = items.find((i) => typeof i === 'string' || (i && i.title));
      return Promise.resolve(first);
    },
    showErrorMessage(m, ...items) {
      send('message', { level: 'error', text: String(m) });
      const first = items.find((i) => typeof i === 'string' || (i && i.title));
      return Promise.resolve(first);
    },
    showQuickPick(items, options, token) {
      return Promise.resolve(items).then((list) => {
        if (!Array.isArray(list) || list.length === 0) return undefined;
        return list[0];
      });
    },
    showInputBox(options, token) {
      return Promise.resolve(options && options.value ? options.value : undefined);
    },
    showOpenDialog(options) {
      if (options && options.defaultUri) {
        return Promise.resolve([options.defaultUri]);
      }
      return Promise.resolve(undefined);
    },
    showSaveDialog(options) {
      if (options && options.defaultUri) {
        return Promise.resolve(options.defaultUri);
      }
      const home = process.env.HOME || (process.env.USERPROFILE || '.');
      const filename = (options && options.filters && Object.keys(options.filters)[0]
        ? 'output.' + Object.keys(options.filters)[0].toLowerCase()
        : 'code.png');
      return Promise.resolve(Uri.file(path.join(home, 'Desktop', filename)));
    },
    showTextDocument(doc) {
      send('editor/open', { uri: doc.uri.toString() });
      const ed = new TextEditor(doc);
      activeEditor = ed;
      visibleEditors = [ed];
      onDidChangeActiveTextEditor.fire(ed);
      onDidChangeVisibleTextEditors.fire(visibleEditors);
      return Promise.resolve(ed);
    },
    createTextEditorDecorationType(options) {
      const key = 'dec_' + nextDecorationId++;
      send('decoration/create', { key, options: options || {} });
      return { key, dispose() { send('decoration/dispose', { key }); } };
    },
    createOutputChannel(name, options) {
      const c = outputChannel(name);
      outputChannels.set(name, c);
      return c;
    },
    createTerminal(options) {
      const id = 'term_' + (nextTerminalId++);
      let name = 'Terminal';
      let cwd = undefined;
      if (typeof options === 'string') {
        name = options;
      } else if (options && typeof options === 'object') {
        name = options.name || 'Terminal';
        if (options.cwd) {
          cwd = typeof options.cwd === 'string' ? options.cwd : (options.cwd.fsPath || String(options.cwd));
        }
      }
      const term = new Terminal(id, name, typeof options === 'object' ? options : { name });
      terminalInstances.set(id, term);
      activeTerminalInstance = term;
      send('terminal/create', { id, name, cwd });
      onDidOpenTerminal.fire(term);
      onDidChangeActiveTerminal.fire(term);
      return term;
    },
    createStatusBarItem(idOrAlignment, alignmentOrPriority, priority) {
      let id, align = StatusBarAlignment.Left, prio = 0;
      if (typeof idOrAlignment === 'string') {
        id = idOrAlignment;
        align = alignmentOrPriority || StatusBarAlignment.Left;
        prio = priority || 0;
      } else {
        align = idOrAlignment || StatusBarAlignment.Left;
        prio = alignmentOrPriority || 0;
      }
      return new vscode.StatusBarItem(id, align, prio);
    },
    registerTreeDataProvider(id, provider) {
      providers.set('tree:' + id, { provider });
      if (provider.onDidChangeTreeData) {
        provider.onDidChangeTreeData(() => send('tree/changed', { viewId: id }));
      }
      return new Disposable(() => providers.delete('tree:' + id));
    },
    createTreeView(id, options) {
      if (options && options.treeDataProvider) {
        this.registerTreeDataProvider(id, options.treeDataProvider);
      }
      return {
        id,
        title: (options && options.title) || id,
        badge: undefined,
        message: undefined,
        visible: true,
        selection: [],
        reveal() {},
        onDidChangeSelection: new EventEmitter().event,
        onDidChangeVisibility: new EventEmitter().event,
        dispose() { providers.delete('tree:' + id); },
      };
    },
    createWebviewPanel(viewType, title, showOptions, options) {
      const panelId = 'wv_' + (nextWebviewId++);
      const onDidReceiveMessage = new EventEmitter();
      const onDidDispose = new EventEmitter();
      let _html = '';
      send('webview/create', { id: panelId, viewType, title, showOptions, options });
      const panelObj = {
        viewType,
        title,
        visible: true,
        active: true,
        webview: {
          get html() { return _html; },
          set html(val) {
            _html = String(val);
            send('webview/setHtml', { id: panelId, html: _html });
          },
          options: options || {},
          get cspSource() {
            return webviewPort
              ? `http://127.0.0.1:${webviewPort} http://127.0.0.1:* https: data: 'unsafe-inline' 'unsafe-eval' vscode-resource:`
              : `'unsafe-inline' 'unsafe-eval' *`;
          },
          onDidReceiveMessage: onDidReceiveMessage.event,
          postMessage: (message) => {
            send('webview/postMessage', { id: panelId, message });
            return Promise.resolve(true);
          },
          asWebviewUri: (uri) => {
            if (!uri) return '';
            let p = '';
            if (typeof uri === 'string') {
              p = uri.startsWith('file://') ? uri.slice(7) : uri;
            } else if (uri.fsPath) {
              p = uri.fsPath;
            } else if (uri.path) {
              p = uri.path;
            }
            if (webviewPort && p) {
              const urlStr = `http://127.0.0.1:${webviewPort}/file?path=${encodeURIComponent(p)}`;
              return {
                toString() { return urlStr; },
                toJSON() { return urlStr; },
                scheme: 'http',
                authority: `127.0.0.1:${webviewPort}`,
                path: '/file',
                query: `path=${encodeURIComponent(p)}`,
                fragment: '',
                fsPath: p,
              };
            }
            return uri;
          },
        },
        reveal(viewColumn, preserveFocus) {
          send('webview/reveal', { id: panelId, viewColumn, preserveFocus });
        },
        dispose() {
          send('webview/dispose', { id: panelId });
          webviewPanels.delete(panelId);
          onDidDispose.fire();
        },
        onDidDispose: onDidDispose.event,
      };
      webviewPanels.set(panelId, { panel: panelObj, onDidReceiveMessage, onDidDispose });
      return panelObj;
    },
    registerWebviewViewProvider(id, provider) {
      providers.set('webview:' + id, provider);
      return new Disposable(() => providers.delete('webview:' + id));
    },
    registerCustomEditorProvider(viewType, provider) {
      providers.set('customEditor:' + viewType, provider);
      return new Disposable(() => providers.delete('customEditor:' + viewType));
    },
    registerUriHandler(handler) {
      commands.set('uriHandler', handler.handleUri && handler.handleUri.bind(handler));
      return new Disposable(() => commands.delete('uriHandler'));
    },
    withProgress(options, task) {
      const reporter = { report(v) {} };
      return task(reporter, CancellationToken.None);
    },
    tabGroups: {
      all: [],
      activeTabGroup: undefined,
      onDidChangeTabs: new EventEmitter().event,
      onDidChangeTabGroups: new EventEmitter().event,
      close: async () => true,
    },
  },

  // -------------------------------------------------------------- commands --
  commands: {
    registerCommand(id, callback, thisArg) {
      const bound = thisArg ? callback.bind(thisArg) : callback;
      commands.set(id, bound);
      return new Disposable(() => commands.delete(id));
    },
    registerTextEditorCommand(id, callback, thisArg) {
      return vscode.commands.registerCommand(id, (...args) => {
        if (!activeEditor) return;
        return callback.call(thisArg, activeEditor, {
          insert: () => {}, delete: () => {}, replace: () => {}, setEndOfLine: () => {},
        }, ...args);
      });
    },
    async executeCommand(id, ...args) {
      if (id === 'setContext') {
        const [k, v] = args;
        contextKeys.set(k, v);
        return true;
      }
      for (const ext of extensions.values()) {
        if (!ext.activated && (ext.packageJSON.activationEvents || []).includes('onCommand:' + id)) {
          activate(ext);
        }
      }
      if (commands.has(id)) {
        return commands.get(id)(...args);
      }
      if (id === 'vscode.open') {
        const [targetUri] = args;
        if (targetUri) {
          send('editor/open', { uri: targetUri.toString() });
          return true;
        }
      }
      if (id === 'editor.action.formatDocument' && activeEditor) {
        const provider = providerFor('formatting', activeEditor.document);
        if (provider) return provider.provider.provideDocumentFormattingEdits(activeEditor.document, { tabSize: 4, insertSpaces: true }, CancellationToken.None);
      }
      if (id === 'editor.action.clipboardCopyWithSyntaxHighlightingAction') {
        send('clipboard/copySelection', {});
        return true;
      }
      if (id === 'workbench.action.files.save') {
        if (activeEditor) await activeEditor.document.save();
        return true;
      }
      return request('command/execute', { id, args });
    },
    getCommands() { return Promise.resolve(Array.from(commands.keys())); },
  },

  // ------------------------------------------------------------- languages --
  languages: {
    createDiagnosticCollection(name) { return new DiagnosticCollection(name); },
    get onDidChangeDiagnostics() { return new EventEmitter().event; },
    getDiagnostics(uri) { return []; },
    getLanguages() {
      return Promise.resolve(['plaintext', 'javascript', 'typescript', 'python', 'rust', 'json', 'html', 'css', 'yaml', 'markdown', 'c', 'cpp', 'go', 'java', 'csharp']);
    },
    match(selector, document) { return matches(selector, document) ? 10 : 0; },
    setTextDocumentLanguage(document, id) { document.languageId = id; return Promise.resolve(document); },
    setLanguageConfiguration(id, configuration) {
      providers.set('config:' + id, configuration);
      return new Disposable(() => providers.delete('config:' + id));
    },
    registerCompletionItemProvider(selector, provider, ...triggers) {
      return register('completion', selector, provider, { triggers });
    },
    registerHoverProvider(selector, provider) { return register('hover', selector, provider); },
    registerDefinitionProvider(selector, provider) { return register('definition', selector, provider); },
    registerDeclarationProvider(selector, provider) { return register('declaration', selector, provider); },
    registerTypeDefinitionProvider(selector, provider) { return register('typeDefinition', selector, provider); },
    registerImplementationProvider(selector, provider) { return register('implementation', selector, provider); },
    registerReferenceProvider(selector, provider) { return register('reference', selector, provider); },
    registerDocumentHighlightProvider(selector, provider) { return register('highlights', selector, provider); },
    registerMultiDocumentHighlightProvider(selector, provider) { return register('multiHighlights', selector, provider); },
    registerDocumentSymbolProvider(selector, provider) { return register('symbols', selector, provider); },
    registerWorkspaceSymbolProvider(provider) { return register('workspaceSymbols', '*', provider); },
    registerCodeActionsProvider(selector, provider) { return register('codeActions', selector, provider); },
    registerCodeLensProvider(selector, provider) { return register('codeLens', selector, provider); },
    registerDocumentFormattingEditProvider(selector, provider) { return register('formatting', selector, provider); },
    registerDocumentRangeFormattingEditProvider(selector, provider) { return register('rangeFormatting', selector, provider); },
    registerOnTypeFormattingEditProvider(selector, provider, ...triggers) { return register('onTypeFormatting', selector, provider, { triggers }); },
    registerRenameProvider(selector, provider) { return register('rename', selector, provider); },
    registerDocumentSemanticTokensProvider(selector, provider, legend) { providers.set('semanticLegend', legend); return register('semanticTokens', selector, provider); },
    registerDocumentRangeSemanticTokensProvider(selector, provider) { return register('semanticTokensRange', selector, provider); },
    registerInlayHintsProvider(selector, provider) { return register('inlayHints', selector, provider); },
    registerSignatureHelpProvider(selector, provider) { return register('signature', selector, provider); },
    registerFoldingRangeProvider(selector, provider) { return register('folding', selector, provider); },
    registerSelectionRangeProvider(selector, provider) { return register('selectionRanges', selector, provider); },
    registerCallHierarchyProvider(selector, provider) { return register('callHierarchy', selector, provider); },
    registerTypeHierarchyProvider(selector, provider) { return register('typeHierarchy', selector, provider); },
    registerLinkedEditingRangeProvider(selector, provider) { return register('linkedEditing', selector, provider); },
    registerColorProvider(selector, provider) { return register('colors', selector, provider); },
    registerDocumentLinkProvider(selector, provider) { return register('links', selector, provider); },
    registerInlineCompletionItemProvider(selector, provider) { return register('inlineCompletion', selector, provider); },
    registerEvaluatableExpressionProvider(selector, provider) { return register('evaluatable', selector, provider); },
    registerDocumentDropEditProvider(selector, provider) { return register('drop', selector, provider); },
    registerDocumentPasteEditProvider(selector, provider) { return register('paste', selector, provider); },
    createLanguageStatusItem(id, selector) { return { id, selector, severity: 0, busy: false, command: undefined, text: '', detail: '', dispose() {} }; },
  },

  // ------------------------------------------------------------ extensions --
  extensions: {
    get all() { return Array.from(extensions.values()); },
    getExtension(id) { return extensions.get(String(id).toLowerCase()); },
    get onDidChange() { return new EventEmitter().event; },
  },

  // ------------------------------------------------------------------- env --
  env: {
    appName: 'Coiman IDE',
    appRoot: process.cwd(),
    appHost: 'desktop',
    language: 'en',
    machineId: 'coiman',
    sessionId: 'coiman',
    uriScheme: 'coiman',
    uiKind: UIKind.Desktop,
    remoteName: undefined,
    shell: process.env.SHELL || (process.platform === 'win32' ? 'powershell.exe' : 'sh'),
    isTelemetryEnabled: false,
    clipboard: {
      readText: () => Promise.resolve(''),
      writeText: () => Promise.resolve(),
    },
    openExternal(uri) { send('open-external', { uri: uri.toString() }); return Promise.resolve(true); },
    asExternalUri(uri) { return Promise.resolve(uri); },
    asWebviewUri(uri) { return Promise.resolve(uri); },
    createTelemetryLogger() {
      return {
        logUsage() {}, logError() {}, logInfo() {}, info() {}, warn() {}, error() {}, debug() {}, trace() {},
        dispose() {}, onDidChangeEnablement: () => new Disposable(),
      };
    },
    onDidChangeLogLevel: new EventEmitter().event,
  },

  // --------------------------------------------------------------- stubs --
  l10n: {
    bundle: undefined,
    uri: undefined,
    t(message, ...args) {
      if (args.length && typeof args[0] === 'object' && args[0].message) args = args[0].args || [];
      if (Array.isArray(args)) {
        args.forEach((v, i) => { message = String(message).split('{' + i + '}').join(String(v)); });
      }
      return message;
    },
  },
  debug: {
    get activeDebugSession() { return undefined; },
    get activeDebugConsole() { return { append() {}, appendLine() {} }; },
    get breakpoints() { return []; },
    onDidStartDebugSession: new EventEmitter().event,
    onDidTerminateDebugSession: new EventEmitter().event,
    onDidChangeActiveDebugSession: new EventEmitter().event,
    registerDebugAdapterDescriptorFactory() { return new Disposable(); },
    registerDebugConfigurationProvider() { return new Disposable(); },
    registerDebugAdapterTrackerFactory() { return new Disposable(); },
    startDebugging() { return Promise.resolve(true); },
    stopDebugging() { return Promise.resolve(); },
    addBreakpoints() {}, removeBreakpoints() {},
    asDebugSourceUri(uri) { return uri; },
  },
  tasks: {
    registerTaskProvider(type, provider) {
      taskProviders.set(type, provider);
      return new Disposable(() => taskProviders.delete(type));
    },
    async fetchTasks(filter) {
      const all = [];
      for (const [type, p] of taskProviders.entries()) {
        if (!filter || !filter.type || filter.type === type) {
          try {
            const list = await p.provideTasks();
            if (Array.isArray(list)) all.push(...list);
          } catch (e) {
            log('task provider error: ' + e);
          }
        }
      }
      return all;
    },
    async executeTask(task) {
      onDidStartTask.fire({ execution: { task } });
      let cmd = '';
      if (task.execution instanceof ShellExecution) {
        cmd = task.execution.commandLine || (task.execution.command + ' ' + (task.execution.args || []).join(' '));
      } else if (task.execution instanceof ProcessExecution) {
        cmd = task.execution.process + ' ' + (task.execution.args || []).join(' ');
      }
      if (cmd) {
        const term = vscode.window.createTerminal({ name: task.name });
        term.show();
        term.sendText(cmd);
      }
      onDidEndTask.fire({ execution: { task } });
      return { task };
    },
    onDidStartTask: onDidStartTask.event,
    onDidEndTask: onDidEndTask.event,
    onDidStartTaskProcess: onDidStartTaskProcess.event,
    onDidEndTaskProcess: onDidEndTaskProcess.event,
  },
  scm: {
    createSourceControl(id, label, rootUri) {
      return {
        id, label, rootUri,
        createResourceGroup(gid, glabel) { return { id: gid, label: glabel, resourceStates: [], dispose() {} }; },
        dispose() {},
      };
    },
  },
  comments: {
    createCommentController(id, label) {
      return { id, label, createCommentThread() { return { dispose() {} }; }, dispose() {} };
    },
  },
  tests: {
    createTestController(id, label) {
      return {
        id, label, items: { add() {}, replace() {}, delete() {}, get() {}, forEach() {} },
        createTestRun() { return { passed() {}, failed() {}, errored() {}, skipped() {}, end() {}, appendOutput() {} }; },
        dispose() {},
      };
    },
  },
  authentication: {
    getSession() { return Promise.resolve(undefined); },
    registerAuthenticationProvider() { return new Disposable(); },
    onDidChangeSessions: new EventEmitter().event,
  },
  notebooks: {
    createNotebookController(id, notebookType, label) {
      return {
        id, notebookType, label,
        createNotebookCellExecution: () => ({ start() {}, end() {}, clearOutput() {}, appendOutput() {}, replaceOutput() {} }),
        dispose() {},
      };
    },
    registerNotebookController(id, notebookType, label, controller) { return new Disposable(); },
    registerNotebookSerializer() { return new Disposable(); },
    registerNotebookCellStatusBarItemProvider() { return new Disposable(); },
  },
  lm: {
    registerTool(name, tool) { commands.set('lm.tool.' + name, tool.invoke && tool.invoke.bind(tool)); return new Disposable(() => commands.delete('lm.tool.' + name)); },
    registerLanguageModelChatProvider(vendor, provider) { return new Disposable(); },
    selectChatModels() { return Promise.resolve([]); },
    showChatModelPicker() { return Promise.resolve(undefined); },
  },
  chat: {
    createChatParticipant(id, handler) { commands.set('chat.' + id, handler); return { id, iconPath: undefined, dispose() { commands.delete('chat.' + id); } }; },
  },
  terminal: { registerTerminalProfileProvider() { return new Disposable(); } },
  ai: { registerAIProvider() { return new Disposable(); } },
};

function register(kind, selector, provider, extra = {}) {
  const list = providers.get(kind) || [];
  const entry = { selector, provider, ...extra };
  list.push(entry);
  providers.set(kind, list);
  return new Disposable(() => {
    const i = list.indexOf(entry);
    if (i >= 0) list.splice(i, 1);
  });
}

// ---------------------------------------------------------------- loading ----
global.vscode = vscode;
const originalLoad = Module._load;
Module._load = function (requestName, parent, isMain) {
  if (requestName === 'vscode') return vscode;
  if (requestName === 'vscode/package.json') return { name: 'vscode', version: '1.99.0' };
  return originalLoad.apply(this, arguments);
};

try {
  const nodeModule = require('node:module');
  if (typeof nodeModule.registerHooks === 'function') {
    const keys = Object.keys(vscode);
    const namedExports = keys
      .filter((k) => /^[a-zA-Z_$][a-zA-Z0-9_$]*$/.test(k))
      .map((k) => `export const ${k} = globalThis.vscode['${k}'];`)
      .join('\n');
    const code = `
const _v = globalThis.vscode;
export default _v;
${namedExports}
`;
    const dataUri = 'data:text/javascript;base64,' + Buffer.from(code).toString('base64');
    nodeModule.registerHooks({
      resolve(specifier, context, nextResolve) {
        if (specifier === 'vscode' || specifier === 'vscode/package.json') {
          return { url: dataUri, shortCircuit: true };
        }
        return nextResolve(specifier, context);
      },
    });
  }
} catch (e) {
  log('registerHooks error: ' + String(e));
}

async function activate(extension) {
  if (!extension || extension.activated) return;
  extension.activated = true;
  try {
    const main = extension.main;
    if (!main) return;
    let file = path.isAbsolute(main) ? main : path.resolve(extension.dir, main);
    try {
      file = require.resolve(file);
    } catch (e) {
      if (!fs.existsSync(file)) return;
    }
    delete require.cache[file];
    let mod;
    try {
      mod = require(file);
    } catch (err) {
      if (err && (err.code === 'ERR_REQUIRE_ESM' || String(err).includes('Cannot use import statement') || String(err).includes('ERR_REQUIRE_ESM'))) {
        const url = require('url');
        mod = await import(url.pathToFileURL(file).href);
      } else {
        throw err;
      }
    }
    const activateFn = (mod && typeof mod.activate === 'function') ? mod.activate : (mod && mod.default && typeof mod.default.activate === 'function' ? mod.default.activate : null);
    if (typeof activateFn === 'function') {
      const result = activateFn(extensionContext(extension));
      if (result && typeof result.then === 'function') {
        result.then((value) => { extension.exports = value; extension.isActive = true; }).catch((e) => log('activate failed: ' + e));
      } else {
        extension.exports = result;
        extension.isActive = true;
      }
    }
  } catch (e) {
    log('activate failed for ' + extension.id + ': ' + (e && e.stack ? e.stack : String(e)));
  }
}

function registerExtensionRecord(ext) {
  const record = {
    id: ext.id,
    idLower: ext.id.toLowerCase(),
    name: ext.name || ext.id,
    dir: ext.dir,
    extensionPath: ext.dir,
    extensionUri: Uri.file(ext.dir),
    main: ext.main || '',
    version: ext.version || '',
    packageJSON: ext.package || {},
    exports: undefined,
    isActive: false,
    activated: false,
  };
  record.activate = () => activate(record);
  extensions.set(record.idLower, record);

  const contributes = record.packageJSON.contributes || {};
  let events = record.packageJSON.activationEvents;
  if (!events || !Array.isArray(events)) {
    events = [];
    if (contributes.commands) {
      for (const c of contributes.commands) if (c && c.command) events.push('onCommand:' + c.command);
    }
    if (contributes.languages) {
      for (const l of contributes.languages) if (l && l.id) events.push('onLanguage:' + l.id);
    }
    if (contributes.views) {
      for (const views of Object.values(contributes.views)) {
        if (Array.isArray(views)) {
          for (const v of views) if (v && v.id) events.push('onView:' + v.id);
        }
      }
    }
    record.packageJSON.activationEvents = events;
  }

  const categories = Array.isArray(contributes.configuration) ? contributes.configuration : [contributes.configuration];
  for (const category of categories) {
    for (const [key, definition] of Object.entries((category && category.properties) || {})) {
      if (definition && Object.prototype.hasOwnProperty.call(definition, 'default') && !config.has(key)) {
        config.set(key, definition.default);
      }
    }
  }
  for (const [key, value] of Object.entries(contributes.configurationDefaults || {})) {
    if (!config.has(key)) config.set(key, value);
  }
  return record;
}

function invoke(kind, document, extra, id) {
  const p = providerFor(kind, document);
  if (!p) { answer(id, null); return; }
  try {
    let result;
    const token = CancellationToken.None;
    if (kind === 'completion') {
      result = p.provider.provideCompletionItems(document, extra.position, token, extra.context || { triggerKind: 0 });
    } else if (kind === 'hover') {
      result = p.provider.provideHover(document, extra.position, token);
    } else if (kind === 'definition') {
      result = p.provider.provideDefinition(document, extra.position, token);
    } else if (kind === 'declaration') {
      result = p.provider.provideDeclaration(document, extra.position, token);
    } else if (kind === 'typeDefinition') {
      result = p.provider.provideTypeDefinition(document, extra.position, token);
    } else if (kind === 'implementation') {
      result = p.provider.provideImplementation(document, extra.position, token);
    } else if (kind === 'reference') {
      result = p.provider.provideReferences(document, extra.position, extra.context || { includeDeclaration: true }, token);
    } else if (kind === 'highlights') {
      result = p.provider.provideDocumentHighlights(document, extra.position, token);
    } else if (kind === 'symbols') {
      result = p.provider.provideDocumentSymbols(document, token);
    } else if (kind === 'workspaceSymbols') {
      result = p.provider.provideWorkspaceSymbols(extra.query || '', token);
    } else if (kind === 'codeActions') {
      result = p.provider.provideCodeActions(document, extra.range, extra.context || {}, token);
    } else if (kind === 'codeLens') {
      result = p.provider.provideCodeLenses(document, token);
    } else if (kind === 'formatting') {
      result = p.provider.provideDocumentFormattingEdits(document, extra.options || { tabSize: 4, insertSpaces: true }, token);
    } else if (kind === 'rangeFormatting') {
      result = p.provider.provideDocumentRangeFormattingEdits(document, extra.range, extra.options || { tabSize: 4, insertSpaces: true }, token);
    } else if (kind === 'onTypeFormatting') {
      result = p.provider.provideOnTypeFormattingEdits(document, extra.position, extra.ch || '', extra.options || {}, token);
    } else if (kind === 'rename') {
      result = p.provider.provideRenameEdits(document, extra.position, extra.newName || '', token);
    } else if (kind === 'signature') {
      result = p.provider.provideSignatureHelp(document, extra.position, token, extra.context || {});
    } else if (kind === 'inlayHints') {
      result = p.provider.provideInlayHints(document, extra.range, token);
    } else if (kind === 'semanticTokens') {
      result = p.provider.provideDocumentSemanticTokens(document, token);
    } else if (kind === 'folding') {
      result = p.provider.provideFoldingRanges(document, {}, token);
    } else if (kind === 'selectionRanges') {
      result = p.provider.provideSelectionRanges(document, extra.positions || [], token);
    } else if (kind === 'colors') {
      result = p.provider.provideDocumentColors(document, token);
    } else if (kind === 'links') {
      result = p.provider.provideDocumentLinks(document, token);
    } else if (kind === 'inlineCompletion') {
      result = p.provider.provideInlineCompletionItems(document, extra.position, extra.context || {}, token);
    } else if (kind === 'evaluatable') {
      result = p.provider.provideEvaluatableExpression(document, extra.position, token);
    }

    Promise.resolve(result)
      .then((r) => {
        if (r === undefined) r = null;
        if (r && typeof r === 'object') {
          // Normalize completions if CompletionList
          if (r.items && Array.isArray(r.items)) r = r.items;
        }
        answer(id, r);
      })
      .catch((e) => answer(id, null, String(e)));
  } catch (e) {
    answer(id, null, String(e));
  }
}

const DEFAULT_LANGUAGE_COMMENTS = {
  javascript: { lineComment: '//', blockComment: ['/*', '*/'] },
  javascriptreact: { lineComment: '//', blockComment: ['/*', '*/'] },
  typescript: { lineComment: '//', blockComment: ['/*', '*/'] },
  typescriptreact: { lineComment: '//', blockComment: ['/*', '*/'] },
  python: { lineComment: '#' },
  rust: { lineComment: '//', blockComment: ['/*', '*/'] },
  c: { lineComment: '//', blockComment: ['/*', '*/'] },
  cpp: { lineComment: '//', blockComment: ['/*', '*/'] },
  csharp: { lineComment: '//', blockComment: ['/*', '*/'] },
  go: { lineComment: '//', blockComment: ['/*', '*/'] },
  java: { lineComment: '//', blockComment: ['/*', '*/'] },
  html: { blockComment: ['<!--', '-->'] },
  xml: { blockComment: ['<!--', '-->'] },
  css: { blockComment: ['/*', '*/'] },
  scss: { lineComment: '//', blockComment: ['/*', '*/'] },
  less: { lineComment: '//', blockComment: ['/*', '*/'] },
  json: { lineComment: '//', blockComment: ['/*', '*/'] },
  jsonc: { lineComment: '//', blockComment: ['/*', '*/'] },
  yaml: { lineComment: '#' },
  shellscript: { lineComment: '#' },
  bash: { lineComment: '#' },
  sh: { lineComment: '#' },
  toml: { lineComment: '#' },
  sql: { lineComment: '--', blockComment: ['/*', '*/'] },
  markdown: { blockComment: ['<!--', '-->'] },
  php: { lineComment: '//', blockComment: ['/*', '*/'] },
  ruby: { lineComment: '#' },
};

function handle(msg) {
  if (msg.jsonrpc === '2.0' && msg.id !== undefined && msg.method === undefined) {
    const fn = waiting.get(msg.id);
    if (fn) { waiting.delete(msg.id); fn(msg.result); }
    return;
  }
  const params = msg.params || {};
  switch (msg.method) {
    case 'initialize': {
      if (params.webviewPort) {
        webviewPort = Number(params.webviewPort);
      }
      workspaceFolders = (params.workspaceFolders || []).map((f, i) => ({
        uri: Uri.parse(f.uri || f),
        name: f.name || path.basename(Uri.parse(f.uri || f).fsPath),
        index: f.index !== undefined ? f.index : i,
      }));
      for (const [key, value] of Object.entries(params.config || {})) config.set(key, value);
      for (const ext of params.extensions || []) {
        registerExtensionRecord(ext);
      }
      for (const [langId, comments] of Object.entries(DEFAULT_LANGUAGE_COMMENTS)) {
        if (!providers.has('config:' + langId)) {
          providers.set('config:' + langId, { comments });
        }
      }
      for (const ext of extensions.values()) {
        const events = ext.packageJSON.activationEvents || [];
        if (events.includes('onStartupFinished') || events.includes('*')) {
          activate(ext);
        }
      }
      send('ready', { extensions: extensions.size });
      break;
    }
    case 'extension/register': {
      if (params.extension) {
        const record = registerExtensionRecord(params.extension);
        const events = record.packageJSON.activationEvents || [];
        if (events.includes('onStartupFinished') || events.includes('*')) {
          activate(record);
        }
        answer(msg.id, true);
      }
      break;
    }
    case 'workspace/setFolders': {
      workspaceFolders = (params.workspaceFolders || []).map((f, i) => ({
        uri: Uri.parse(f.uri || f),
        name: f.name || path.basename(Uri.parse(f.uri || f).fsPath),
        index: f.index !== undefined ? f.index : i,
      }));
      onDidChangeWorkspaceFolders.fire({ added: workspaceFolders, removed: [] });
      break;
    }
    case 'document/open': {
      const doc = new TextDocument(params.document);
      documents.set(doc.uri.toString(), doc);
      activeEditor = new TextEditor(doc);
      visibleEditors = [activeEditor];
      for (const ext of extensions.values()) {
        if (!ext.activated) {
          const events = ext.packageJSON.activationEvents || [];
          const languages = (ext.packageJSON.contributes && ext.packageJSON.contributes.languages) || [];
          if (events.some((e) => e === 'onLanguage:' + doc.languageId || e === 'onLanguage')
            || languages.some((l) => l && l.id === doc.languageId)) {
            activate(ext);
          }
        }
      }
      onDidOpenTextDocument.fire(doc);
      onDidChangeActiveTextEditor.fire(activeEditor);
      break;
    }
    case 'document/change': {
      let doc = documents.get(params.uri);
      if (!doc) {
        doc = new TextDocument({ uri: Uri.parse(params.uri), languageId: params.languageId || 'plaintext', text: params.text || '' });
        documents.set(params.uri, doc);
      } else {
        doc._update(params.text, params.version);
      }
      activeEditor = new TextEditor(doc);
      visibleEditors = [activeEditor];
      break;
    }
    case 'document/close': {
      const doc = documents.get(params.uri);
      if (doc) {
        doc.isClosed = true;
        onDidCloseTextDocument.fire(doc);
        documents.delete(params.uri);
      }
      break;
    }
    case 'document/save': {
      const doc = documents.get(params.uri);
      if (doc) onDidSaveTextDocument.fire(doc);
      break;
    }
    case 'configuration/changed': {
      for (const [key, value] of Object.entries(params.config || {})) config.set(key, value);
      onDidChangeConfiguration.fire({ affectsConfiguration: (section) => Object.keys(params.config || {}).some((k) => k.startsWith(section)) });
      break;
    }
    case 'editor/selection': {
      const doc = documents.get(params.uri);
      if (doc) {
        const start = new Position(params.start.line, params.start.character);
        const end = new Position(params.end.line, params.end.character);
        const sel = new Selection(start, end);
        if (!activeEditor || activeEditor.document.uri.toString() !== doc.uri.toString()) {
          activeEditor = new TextEditor(doc);
          visibleEditors = [activeEditor];
        }
        activeEditor.selection = sel;
        activeEditor.selections = [sel];
        onDidChangeTextEditorSelection.fire({
          textEditor: activeEditor,
          selections: [sel],
          kind: undefined,
        });
      }
      break;
    }
    case 'webview/message': {
      const p = webviewPanels.get(params.id);
      if (p) {
        p.onDidReceiveMessage.fire(params.message);
      }
      break;
    }
    case 'tree/items': {
      for (const ext of extensions.values()) {
        if (!ext.activated) {
          const events = ext.packageJSON.activationEvents || [];
          const contribViews = (ext.packageJSON.contributes && ext.packageJSON.contributes.views) || {};
          const hasView = Object.entries(contribViews).some(([cid, vlist]) =>
            cid === params.viewId || (Array.isArray(vlist) && vlist.some((v) => v && v.id === params.viewId))
          );
          if (events.includes('onView:' + params.viewId) || events.includes('*') || hasView) {
            activate(ext);
          }
        }
      }
      const entry = providers.get('tree:' + params.viewId);
      if (!entry) { answer(msg.id, []); break; }
      Promise.resolve(entry.provider.getChildren(params.element))
        .then((items) => answer(msg.id, (items || []).map((item) => ({
          label: typeof item.label === 'object' ? item.label.label : (item.label || ''),
          description: item.description || '',
          collapsibleState: item.collapsibleState || 0,
        }))))
        .catch(() => answer(msg.id, []));
      break;
    }
    case 'command/execute': {
      for (const ext of extensions.values()) {
        if (!ext.activated) {
          const events = ext.packageJSON.activationEvents || [];
          const cmds = (ext.packageJSON.contributes && ext.packageJSON.contributes.commands) || [];
          const hasCmd = cmds.some((c) => c && c.command === params.id);
          if (events.includes('onCommand:' + params.id) || events.includes('*') || hasCmd) {
            activate(ext);
          }
        }
      }
      Promise.resolve(vscode.commands.executeCommand(params.id, ...(params.args || [])))
        .then((r) => answer(msg.id, r === undefined ? null : r))
        .catch((e) => answer(msg.id, null, String(e)));
      break;
    }
    case 'invoke': {
      const doc = documents.get(params.uri) || new TextDocument({ uri: Uri.parse(params.uri), languageId: params.languageId || 'plaintext', text: params.text || '' });
      invoke(params.kind, doc, params, msg.id);
      break;
    }
    case 'activate/language': {
      for (const ext of extensions.values()) {
        const events = ext.packageJSON.activationEvents || [];
        const languages = (ext.packageJSON.contributes && ext.packageJSON.contributes.languages) || [];
        const matchesLang = events.some((e) => e === 'onLanguage:' + params.language || e === 'onLanguage')
          || languages.some((l) => l && l.id === params.language);
        if (!ext.activated && matchesLang) activate(ext);
      }
      answer(msg.id, true);
      break;
    }
    case 'activate/command': {
      for (const ext of extensions.values()) {
        if (!ext.activated) {
          const events = ext.packageJSON.activationEvents || [];
          const cmds = (ext.packageJSON.contributes && ext.packageJSON.contributes.commands) || [];
          const hasCmd = cmds.some((c) => c && c.command === params.command);
          if (events.includes('onCommand:' + params.command) || events.includes('*') || hasCmd) {
            activate(ext);
          }
        }
      }
      answer(msg.id, true);
      break;
    }
    case 'activate/view': {
      for (const ext of extensions.values()) {
        if (!ext.activated) {
          const events = ext.packageJSON.activationEvents || [];
          const contribViews = (ext.packageJSON.contributes && ext.packageJSON.contributes.views) || {};
          const hasView = Object.entries(contribViews).some(([cid, vlist]) =>
            cid === params.view || (Array.isArray(vlist) && vlist.some((v) => v && v.id === params.view))
          );
          if (events.includes('onView:' + params.view) || events.includes('*') || hasView) {
            activate(ext);
          }
        }
      }
      answer(msg.id, true);
      break;
    }
    case 'activate/extension':
    case 'extension/activate': {
      const ext = extensions.get(String(params.id).toLowerCase());
      if (ext) activate(ext);
      answer(msg.id, !!ext);
      break;
    }
    case 'shutdown': {
      subscriptions.forEach((d) => { try { d.dispose(); } catch (e) {} });
      process.exit(0);
      break;
    }
    default: break;
  }
}

let buffer = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', (chunk) => {
  buffer += chunk;
  let at;
  while ((at = buffer.indexOf('\n')) >= 0) {
    const line = buffer.slice(0, at);
    buffer = buffer.slice(at + 1);
    if (!line.trim()) continue;
    try {
      handle(JSON.parse(line));
    } catch (e) {
      log('bad message: ' + String(e));
    }
  }
});
process.on('disconnect', () => process.exit(0));
send('hello', {});
