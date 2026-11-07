//! What proof assistants' language servers say about the cursor. Their own requests,
//! outside LSP, read here into plain text.

use std::collections::HashMap;

use serde_json::Value;

/// A language whose server knows the proof state at a position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prover {
    Lean,
    Coq,
}

impl Prover {
    pub fn for_extension(extension: &str) -> Option<Self> {
        match extension {
            "lean" => Some(Self::Lean),
            "v" => Some(Self::Coq),
            _ => None,
        }
    }

    /// The request that gives the goals.
    pub fn goals_method(self) -> &'static str {
        match self {
            Self::Lean => "$/lean/plainGoal",
            Self::Coq => "proof/goals",
        }
    }

    /// The request that gives the type expected here, if there's one.
    pub fn expected_type_method(self) -> Option<&'static str> {
        match self {
            Self::Lean => Some("$/lean/plainTermGoal"),
            Self::Coq => None,
        }
    }

    pub fn goals_text(self, answer: &Value) -> String {
        match self {
            Self::Lean => lean_goals(answer),
            Self::Coq => coq_goals(answer),
        }
    }
}

fn lean_goals(answer: &Value) -> String {
    let Some(goals) = answer["goals"].as_array() else { return String::new() };
    if goals.is_empty() {
        return "No goals".to_owned();
    }
    let goals: Vec<&str> = goals.iter().filter_map(Value::as_str).collect();
    goals.join("\n\n")
}

/// The type a Lean term is expected to have.
pub fn lean_expected_type(answer: &Value) -> String {
    answer["goal"].as_str().unwrap_or_default().to_owned()
}

fn coq_goals(answer: &Value) -> String {
    let Some(goals) = answer["goals"]["goals"].as_array() else { return String::new() };
    if goals.is_empty() {
        return "No goals".to_owned();
    }
    let text = |value: &Value| value.as_str().map_or_else(|| value.to_string(), str::to_owned);
    let one = |goal: &Value| {
        let mut lines = Vec::new();
        for hypothesis in goal["hyps"].as_array().into_iter().flatten() {
            let names: Vec<String> = hypothesis["names"].as_array().into_iter().flatten().map(text).collect();
            lines.push(format!("{} : {}", names.join(" "), text(&hypothesis["ty"])));
        }
        lines.push(format!("⊢ {}", text(&goal["ty"])));
        lines.join("\n")
    };
    goals.iter().map(one).collect::<Vec<_>>().join("\n\n")
}

/// For each line of `new`, whether it wasn't in `old`. With nothing to compare to, none.
pub fn changed_lines(old: &str, new: &str) -> Vec<bool> {
    if old.is_empty() {
        return vec![false; new.lines().count()];
    }
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for line in old.lines() {
        *seen.entry(line).or_default() += 1;
    }
    new.lines()
        .map(|line| match seen.get_mut(line) {
            Some(count) if *count > 0 => {
                *count -= 1;
                false
            }
            _ => true,
        })
        .collect()
}

/// Lines of `old` that `new` no longer has.
pub fn removed_lines(old: &str, new: &str) -> Vec<String> {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for line in new.lines() {
        *seen.entry(line).or_default() += 1;
    }
    let mut gone = Vec::new();
    for line in old.lines() {
        match seen.get_mut(line) {
            Some(count) if *count > 0 => *count -= 1,
            _ => gone.push(line.to_owned()),
        }
    }
    gone
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_lean() {
        let answer = json!({"rendered": "```lean\n...\n```", "goals": ["a : Nat\n⊢ a = a", "⊢ True"]});
        assert_eq!(Prover::Lean.goals_text(&answer), "a : Nat\n⊢ a = a\n\n⊢ True");
        assert_eq!(Prover::Lean.goals_text(&json!({"goals": []})), "No goals");
        assert_eq!(Prover::Lean.goals_text(&Value::Null), "");
        assert_eq!(lean_expected_type(&json!({"goal": "⊢ Nat", "range": {}})), "⊢ Nat");
        assert_eq!(lean_expected_type(&Value::Null), "");
    }

    #[test]
    fn reads_coq() {
        let answer = json!({"goals": {"goals": [{"hyps": [{"names": ["n", "m"], "ty": "nat"}], "ty": "n = m"}]}});
        assert_eq!(Prover::Coq.goals_text(&answer), "n m : nat\n⊢ n = m");
        assert_eq!(Prover::Coq.goals_text(&json!({"goals": {"goals": []}})), "No goals");
        assert_eq!(Prover::Coq.goals_text(&json!({"goals": null})), "");
    }

    #[test]
    fn marks_what_changed() {
        assert_eq!(changed_lines("a\n⊢ b", "a\n⊢ c"), [false, true]);
        assert_eq!(changed_lines("", "a\nb"), [false, false]);
        assert_eq!(changed_lines("a", "a\na"), [false, true], "a repeat counts");
        assert_eq!(removed_lines("a\n⊢ b", "a\n⊢ c"), ["⊢ b"]);
        assert!(removed_lines("a", "a\nb").is_empty());
    }

    #[test]
    fn knows_provers() {
        assert_eq!(Prover::for_extension("lean"), Some(Prover::Lean));
        assert_eq!(Prover::for_extension("v"), Some(Prover::Coq));
        assert_eq!(Prover::for_extension("rs"), None);
    }
}
