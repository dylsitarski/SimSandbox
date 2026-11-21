//! What a definition says about itself (ADR 0109): `#` lines before its
//! first statement are the block's summary; `# text` after a statement
//! describes what that line declares; `param NAME … as ROLE` gives the
//! parameter a role. Read before everything else, and taken off the lines
//! so the phases see code only; columns of the code are kept.

use super::statements::definition_error;
use simsandbox_core::block::{BlockSpec, ParamRole};
use simsandbox_core::Result;

/// The notes of a definition.
#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct Notes {
    pub summary: String,
    /// What a line declares (its name) and the text describing it.
    pub descriptions: Vec<(String, String)>,
    pub roles: Vec<(String, ParamRole)>,
}

/// Where a line's comment starts: the first `#` outside double quotes.
fn comment_at(line: &str) -> Option<usize> {
    let mut quoted = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '#' if !quoted => return Some(i),
            _ => {}
        }
    }
    None
}

/// The name a line declares: `param m …`, `input F …`, `state x …`, or an
/// equation's left side (`x = …`, `dx/dt = …`, `x' = …`, `x[n+1] = …`).
fn declared(code: &str) -> Option<String> {
    let t = code.trim();
    let ident = |s: &str| -> Option<String> {
        let n: String = s
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        (!n.is_empty()).then_some(n)
    };
    for word in ["param ", "input ", "state "] {
        if let Some(rest) = t.strip_prefix(word) {
            return ident(rest.trim_start());
        }
    }
    let lhs = t.split_once('=')?.0.trim();
    if let Some(inner) = lhs.strip_prefix('d').and_then(|r| r.strip_suffix("/dt")) {
        return ident(inner.trim());
    }
    ident(lhs)
}

/// The lines without their notes, and the notes. A role that is not one
/// is an error at its line.
pub(super) fn read(lines: &[String]) -> Result<(Vec<String>, Notes)> {
    let mut notes = Notes::default();
    let mut code_lines = Vec::with_capacity(lines.len());
    let mut started = false;
    let mut summary: Vec<String> = Vec::new();
    for (k, line) in lines.iter().enumerate() {
        let (code, comment) = match comment_at(line) {
            Some(i) => (&line[..i], Some(line[i + 1..].trim())),
            None => (line.as_str(), None),
        };
        let mut code = code.trim_end().to_string();
        if code.trim().is_empty() {
            // A comment line: the summary while no statement came yet.
            if let (false, Some(c)) = (started, comment) {
                if !c.is_empty() {
                    summary.push(c.to_string());
                }
            }
            code_lines.push(String::new());
            continue;
        }
        started = true;
        // `param … as ROLE`: the role, then the line without it.
        if code.trim_start().starts_with("param ") {
            if let Some(at) = code.rfind(" as ") {
                let word = code[at + 4..].trim();
                if !word.is_empty() && word.chars().all(|c| c.is_ascii_alphabetic()) {
                    let role = ParamRole::parse(word).ok_or_else(|| {
                        let names: Vec<&str> = ParamRole::ALL.iter().map(|r| r.name()).collect();
                        definition_error(
                            k + 1,
                            at + 5,
                            format!("'{word}' is not a role (one of {})", names.join(", ")),
                        )
                    })?;
                    if let Some(n) = declared(&code) {
                        notes.roles.push((n, role));
                    }
                    code.truncate(at);
                }
            }
        }
        if let (Some(c), Some(n)) = (comment.filter(|c| !c.is_empty()), declared(&code)) {
            notes.descriptions.push((n, c.to_string()));
        }
        code_lines.push(code);
    }
    notes.summary = summary.join("\n");
    Ok((code_lines, notes))
}

/// Put the notes on the block's interface: the summary, each parameter's
/// and port's description, each parameter's role.
pub(super) fn apply(notes: &Notes, spec: &mut BlockSpec) {
    spec.summary.clone_from(&notes.summary);
    for (name, text) in &notes.descriptions {
        for p in spec.params.iter_mut().filter(|p| p.name == *name) {
            p.description.clone_from(text);
        }
        for p in spec.ports.iter_mut().filter(|p| p.name == *name) {
            p.description.clone_from(text);
        }
    }
    for (name, role) in &notes.roles {
        for p in spec.params.iter_mut().filter(|p| p.name == *name) {
            p.role = *role;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    #[test]
    fn notes_are_read_and_taken_off_the_lines() {
        let (code, notes) = read(&lines(
            "# a damped oscillator\n# pushed by F\n\ninput F [N]   # the push\nparam k [N/m] = 2 as tuning  # stiffness\nparam m [kg] = 1\nstart c = file \"a#b.csv\"\ndx/dt = v  # position\nv' = (F - k*x)/m\ny = 2*x [m] # twice\n# a comment between\n",
        ))
        .unwrap();
        assert_eq!(notes.summary, "a damped oscillator\npushed by F");
        assert_eq!(
            notes.descriptions,
            [
                ("F", "the push"),
                ("k", "stiffness"),
                ("x", "position"),
                ("y", "twice")
            ]
            .map(|(a, b)| (a.to_string(), b.to_string()))
        );
        assert_eq!(notes.roles, [("k".to_string(), ParamRole::Tuning)]);
        assert_eq!(code[3], "input F [N]");
        assert_eq!(code[4], "param k [N/m] = 2");
        assert_eq!(code[6], "start c = file \"a#b.csv\"");
        assert_eq!(code[10], "");
    }

    #[test]
    fn a_role_that_is_not_one_is_an_error_at_its_line() {
        let e = read(&lines("param k = 2 as gain")).unwrap_err().to_string();
        assert!(
            e.contains("line 1") && e.contains("'gain' is not a role"),
            "{e}"
        );
    }
}
