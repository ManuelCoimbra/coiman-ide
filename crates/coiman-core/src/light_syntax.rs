//! Coloring by reading words, for languages with no grammar here. It knows comments,
//! strings, numbers and keywords, which is most of what makes code easy to scan.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Comment,
    Text,
    Number,
    Keyword,
    Operator,
}

/// A colored part of a line: its line, and columns in chars.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
}

struct Rules {
    line_comments: &'static [&'static str],
    block_comments: &'static [(&'static str, &'static str)],
    quotes: &'static [char],
    /// Whether a quote can be escaped with a backslash, rather than by doubling it.
    backslash: bool,
    keywords: &'static [&'static str],
    ignore_case: bool,
    /// Characters colored as operators wherever they appear.
    glyphs: &'static str,
    /// Characters that can continue a word beyond letters and digits.
    word_extra: &'static str,
}

const LEAN: Rules = Rules {
    line_comments: &["--"],
    block_comments: &[("/-", "-/")],
    quotes: &['"'],
    backslash: true,
    keywords: &[
        "theorem",
        "lemma",
        "def",
        "abbrev",
        "example",
        "instance",
        "structure",
        "class",
        "inductive",
        "namespace",
        "section",
        "end",
        "open",
        "import",
        "variable",
        "universe",
        "where",
        "with",
        "match",
        "fun",
        "let",
        "have",
        "show",
        "from",
        "by",
        "at",
        "if",
        "then",
        "else",
        "do",
        "return",
        "for",
        "in",
        "deriving",
        "extends",
        "mutual",
        "private",
        "protected",
        "noncomputable",
        "partial",
        "unsafe",
        "macro",
        "syntax",
        "notation",
        "attribute",
        "calc",
        "sorry",
        "Type",
        "Prop",
        "Sort",
    ],
    ignore_case: false,
    glyphs: "",
    word_extra: "_'!?",
};

const AGDA: Rules = Rules {
    line_comments: &["--"],
    block_comments: &[("{-", "-}")],
    quotes: &['"'],
    backslash: true,
    keywords: &[
        "module",
        "where",
        "open",
        "import",
        "data",
        "record",
        "field",
        "constructor",
        "let",
        "in",
        "with",
        "rewrite",
        "postulate",
        "abstract",
        "private",
        "instance",
        "mutual",
        "infix",
        "infixl",
        "infixr",
        "forall",
        "Set",
        "Prop",
        "public",
        "using",
        "hiding",
        "renaming",
        "to",
        "do",
        "syntax",
        "pattern",
        "primitive",
    ],
    ignore_case: false,
    glyphs: "",
    word_extra: "_'-",
};

const COQ: Rules = Rules {
    line_comments: &[],
    block_comments: &[("(*", "*)")],
    quotes: &['"'],
    backslash: false,
    keywords: &[
        "Theorem",
        "Lemma",
        "Definition",
        "Fixpoint",
        "Inductive",
        "Record",
        "Structure",
        "Class",
        "Instance",
        "Proof",
        "Qed",
        "Defined",
        "Admitted",
        "Require",
        "Import",
        "Export",
        "Module",
        "End",
        "Section",
        "Variable",
        "Hypothesis",
        "Notation",
        "Ltac",
        "Example",
        "Corollary",
        "Remark",
        "Fact",
        "forall",
        "exists",
        "fun",
        "match",
        "with",
        "end",
        "let",
        "in",
        "if",
        "then",
        "else",
        "Type",
        "Prop",
        "Set",
    ],
    ignore_case: false,
    glyphs: "",
    word_extra: "_'",
};

const APL: Rules = Rules {
    line_comments: &["⍝"],
    block_comments: &[],
    quotes: &['\''],
    backslash: false,
    keywords: &[
        ":If",
        ":Else",
        ":ElseIf",
        ":EndIf",
        ":For",
        ":In",
        ":EndFor",
        ":While",
        ":EndWhile",
        ":Repeat",
        ":Until",
        ":EndRepeat",
        ":Select",
        ":Case",
        ":EndSelect",
        ":Return",
        ":Namespace",
        ":EndNamespace",
        ":Class",
        ":EndClass",
        ":Trap",
        ":EndTrap",
    ],
    ignore_case: true,
    glyphs: "←→↑↓⍺⍵⍳⍴⍒⍋⌈⌊×÷⌽⍉⊖∊≢≡≠≤≥∧∨⍲⍱⊂⊃⊆∪∩⍎⍕⎕⍞¨⍣⍤∘⍨⌿⍀⋄⍬⌷⊢⊣⍪⍟○!?",
    word_extra: "_∆⍙:",
};

const COBOL: Rules = Rules {
    line_comments: &["*>"],
    block_comments: &[],
    quotes: &['"', '\''],
    backslash: false,
    keywords: &[
        "IDENTIFICATION",
        "ENVIRONMENT",
        "DATA",
        "PROCEDURE",
        "DIVISION",
        "SECTION",
        "PROGRAM-ID",
        "WORKING-STORAGE",
        "LINKAGE",
        "FILE",
        "PIC",
        "PICTURE",
        "VALUE",
        "MOVE",
        "TO",
        "ADD",
        "SUBTRACT",
        "MULTIPLY",
        "DIVIDE",
        "COMPUTE",
        "IF",
        "ELSE",
        "END-IF",
        "PERFORM",
        "END-PERFORM",
        "UNTIL",
        "VARYING",
        "FROM",
        "BY",
        "DISPLAY",
        "ACCEPT",
        "CALL",
        "USING",
        "STOP",
        "RUN",
        "GOBACK",
        "EVALUATE",
        "WHEN",
        "END-EVALUATE",
        "OPEN",
        "CLOSE",
        "READ",
        "WRITE",
        "COPY",
        "THEN",
        "NOT",
        "AND",
        "OR",
        "GIVING",
        "INTO",
        "OF",
    ],
    ignore_case: true,
    glyphs: "",
    word_extra: "-_",
};

fn rules_for(extension: &str) -> Option<&'static Rules> {
    Some(match extension {
        "lean" => &LEAN,
        "agda" => &AGDA,
        "v" => &COQ,
        "apl" | "dyalog" | "aplf" | "apln" => &APL,
        "cbl" | "cob" | "cpy" => &COBOL,
        _ => return None,
    })
}

/// Whether this type of file is colored here.
pub fn colors(extension: &str) -> bool {
    rules_for(extension).is_some()
}

fn starts(chars: &[char], at: usize, text: &str) -> bool {
    text.chars().enumerate().all(|(k, c)| chars.get(at + k) == Some(&c))
}

/// A fixed-form COBOL line with a star or slash in column 7 is all comment.
fn cobol_comment_line(chars: &[char]) -> bool {
    matches!(chars.get(6), Some('*' | '/')) && chars.iter().take(6).all(|c| c.is_ascii_digit() || *c == ' ')
}

/// Colors `text`, a file of this type.
pub fn tokens(extension: &str, text: &str) -> Vec<Token> {
    let Some(rules) = rules_for(extension) else { return Vec::new() };
    let is_cobol = std::ptr::eq(rules, &COBOL);
    let mut out = Vec::new();
    // The close of the comment still open, and how deep: Lean, Agda and Coq nest them.
    let mut open: Option<(&str, &str, usize)> = None;
    for (line, raw) in text.lines().enumerate() {
        let chars: Vec<char> = raw.chars().collect();
        let mut at = 0;
        if is_cobol && cobol_comment_line(&chars) {
            out.push(Token { line, start: 0, end: chars.len(), kind: Kind::Comment });
            continue;
        }
        while at < chars.len() {
            if let Some((opener, close, depth)) = open {
                let start = at;
                let mut depth = depth;
                while at < chars.len() && depth > 0 {
                    if starts(&chars, at, close) {
                        depth -= 1;
                        at += close.chars().count();
                    } else if starts(&chars, at, opener) {
                        depth += 1;
                        at += opener.chars().count();
                    } else {
                        at += 1;
                    }
                }
                out.push(Token { line, start, end: at, kind: Kind::Comment });
                open = (depth > 0).then_some((opener, close, depth));
                continue;
            }
            let c = chars[at];
            if rules.line_comments.iter().any(|m| starts(&chars, at, m)) {
                out.push(Token { line, start: at, end: chars.len(), kind: Kind::Comment });
                break;
            }
            if let Some((opener, close)) = rules.block_comments.iter().find(|(o, _)| starts(&chars, at, o)) {
                let start = at;
                at += opener.chars().count();
                out.push(Token { line, start, end: at, kind: Kind::Comment });
                open = Some((opener, close, 1));
                continue;
            }
            if rules.quotes.contains(&c) {
                let start = at;
                at += 1;
                while at < chars.len() {
                    if rules.backslash && chars[at] == '\\' {
                        at += 2;
                    } else if chars[at] == c {
                        at += 1;
                        // Doubled, it's the quote itself.
                        if rules.backslash || chars.get(at) != Some(&c) {
                            break;
                        }
                        at += 1;
                    } else {
                        at += 1;
                    }
                }
                let end = at.min(chars.len());
                at = end;
                out.push(Token { line, start, end, kind: Kind::Text });
                continue;
            }
            if rules.glyphs.contains(c) {
                out.push(Token { line, start: at, end: at + 1, kind: Kind::Operator });
                at += 1;
                continue;
            }
            let word_char = |ch: char| ch.is_alphanumeric() || rules.word_extra.contains(ch);
            if word_char(c) {
                let start = at;
                while at < chars.len() && word_char(chars[at]) {
                    at += 1;
                }
                let word: String = chars[start..at].iter().collect();
                let keyword = if rules.ignore_case {
                    rules.keywords.iter().any(|k| k.eq_ignore_ascii_case(&word))
                } else {
                    rules.keywords.contains(&word.as_str())
                };
                if keyword {
                    out.push(Token { line, start, end: at, kind: Kind::Keyword });
                } else if word.chars().next().is_some_and(|f| f.is_ascii_digit()) {
                    out.push(Token { line, start, end: at, kind: Kind::Number });
                }
                continue;
            }
            at += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(extension: &str, text: &str) -> Vec<(usize, String, Kind)> {
        tokens(extension, text)
            .into_iter()
            .map(|t| {
                let line = text.lines().nth(t.line).unwrap();
                (t.line, line.chars().skip(t.start).take(t.end - t.start).collect(), t.kind)
            })
            .collect()
    }

    #[test]
    fn colors_lean() {
        let found = kinds("lean", "theorem t : 1 = 1 := by -- done\n/- a /- b -/\nc -/ rfl \"s\\\"x\"");
        assert!(found.contains(&(0, "theorem".into(), Kind::Keyword)));
        assert!(found.contains(&(0, "1".into(), Kind::Number)));
        assert!(found.contains(&(0, "-- done".into(), Kind::Comment)));
        assert!(found.contains(&(1, "/-".into(), Kind::Comment)));
        // Still inside: comments nest.
        assert!(found.contains(&(2, "c -/".into(), Kind::Comment)));
        assert!(found.contains(&(2, "\"s\\\"x\"".into(), Kind::Text)));
        assert!(!found.iter().any(|t| t.1 == "rfl"));
    }

    #[test]
    fn colors_agda_and_coq() {
        let found = kinds("agda", "module M where {- x -}\n-- y");
        assert!(found.contains(&(0, "module".into(), Kind::Keyword)));
        assert!(found.contains(&(1, "-- y".into(), Kind::Comment)));
        let found = kinds("v", "Theorem t : True. (* a *)\nQed.");
        assert!(found.contains(&(0, "Theorem".into(), Kind::Keyword)));
        assert!(found.contains(&(1, "Qed".into(), Kind::Keyword)));
    }

    #[test]
    fn colors_apl() {
        let found = kinds("apl", "x←⍳3 ⍝ note ⍳\n'it''s' ⍴ :If");
        assert!(found.contains(&(0, "←".into(), Kind::Operator)));
        assert!(found.contains(&(0, "3".into(), Kind::Number)));
        assert!(found.contains(&(0, "⍝ note ⍳".into(), Kind::Comment)));
        assert!(found.contains(&(1, "'it''s'".into(), Kind::Text)));
        assert!(found.contains(&(1, ":If".into(), Kind::Keyword)));
    }

    #[test]
    fn colors_cobol() {
        let found = kinds("cbl", "000100* a comment IF\n       move \"x\" to end-if *> note\n");
        assert!(found.contains(&(0, "000100* a comment IF".into(), Kind::Comment)));
        assert!(found.contains(&(1, "move".into(), Kind::Keyword)));
        assert!(found.contains(&(1, "end-if".into(), Kind::Keyword)));
        assert!(found.contains(&(1, "*> note".into(), Kind::Comment)));
    }

    #[test]
    fn leaves_other_files_and_odd_text_alone() {
        assert!(tokens("rs", "fn x").is_empty());
        assert!(colors("lean") && !colors("rs"));
        // Unclosed things end with the line or file, without panicking.
        assert_eq!(kinds("apl", "'abc").len(), 1);
        assert_eq!(kinds("lean", "\"abc\\").len(), 1);
        assert_eq!(kinds("lean", "/-").len(), 1);
    }
}
