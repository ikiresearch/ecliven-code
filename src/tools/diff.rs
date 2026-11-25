use similar::{ChangeTag, TextDiff};

pub fn unified_diff(old_label: &str, new_label: &str, old_text: &str, new_text: &str) -> String {
    let diff = TextDiff::from_lines(old_text, new_text);
    let mut out = String::new();

    out.push_str(&format!("--- {old_label}\n"));
    out.push_str(&format!("+++ {new_label}\n"));

    for group in diff.grouped_ops(3) {
        let first = &group[0];
        let last = &group[group.len() - 1];
        let old_start = first.old_range().start + 1;
        let old_len = last.old_range().end - first.old_range().start;
        let new_start = first.new_range().start + 1;
        let new_len = last.new_range().end - first.new_range().start;

        out.push_str(&format!(
            "@@ -{old_start},{old_len} +{new_start},{new_len} @@\n"
        ));

        for op in group {
            for change in diff.iter_changes(&op) {
                let sign = match change.tag() {
                    ChangeTag::Delete => "-",
                    ChangeTag::Insert => "+",
                    ChangeTag::Equal => " ",
                };
                out.push_str(sign);
                out.push_str(change.value());
                if change.missing_newline() {
                    out.push('\n');
                }
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_unified_diff_with_hunk_headers() {
        let old = "fn main() {\n    println!(\"old\");\n}\n";
        let new = "fn main() {\n    println!(\"new\");\n}\n";
        let diff = unified_diff("a/main.rs", "b/main.rs", old, new);

        assert!(diff.contains("--- a/main.rs"));
        assert!(diff.contains("+++ b/main.rs"));
        assert!(diff.contains("-    println!(\"old\");"));
        assert!(diff.contains("+    println!(\"new\");"));
    }
}
