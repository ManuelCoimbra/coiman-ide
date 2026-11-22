//! Colors for `.http` and `.rest` files (requests the HTTP Client extension sends).

/// A syntax definition for `.http` files.
pub const SYNTAX: &str = r#"%YAML 1.2
---
name: HTTP Request
file_extensions: [http, rest]
scope: source.http
contexts:
  main:
    - match: '^\s*###.*$'
      scope: comment.line.separator.http markup.heading.http
    - match: '^\s*(#|//)\s*@\w+.*$'
      scope: keyword.other.directive.http
    - match: '^\s*(#|//).*$'
      scope: comment.line.http
    - match: '^\s*(@)([\w.-]+)\s*(=)'
      captures:
        1: punctuation.definition.variable.http
        2: variable.other.http
        3: keyword.operator.assignment.http
    - match: '^\s*(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS|TRACE|CONNECT)\b'
      scope: keyword.control.method.http
    - match: '\bHTTP/[\d.]+\b'
      scope: constant.language.version.http
    - match: '^\s*([\w-]+)(:)\s'
      captures:
        1: entity.name.tag.header.http
        2: punctuation.separator.http
    - match: '\{\{'
      scope: punctuation.definition.template.begin.http
      push: template
    - match: '"(?:[^"\\]|\\.)*"(?=\s*:)'
      scope: support.type.property-name.json.http
    - match: '"(?:[^"\\]|\\.)*"'
      scope: string.quoted.double.http
    - match: '\b(true|false|null)\b'
      scope: constant.language.http
    - match: '-?\b\d+(\.\d+)?([eE][-+]?\d+)?\b'
      scope: constant.numeric.http
  template:
    - meta_scope: variable.parameter.template.http
    - match: '\}\}'
      scope: punctuation.definition.template.end.http
      pop: true
"#;
