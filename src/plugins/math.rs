use super::{Entry, EntryKind, Plugin};

/// Holds the clipboard command so `=` results can be copied without hardwiring
/// `wl-copy`; `query` doesn't use it.
pub struct MathPlugin {
    pub clipboard: Vec<String>,
}

impl MathPlugin {
    pub fn new(clipboard: Vec<String>) -> Self {
        Self { clipboard }
    }
}

impl Plugin for MathPlugin {
    fn name(&self) -> &str {
        "math"
    }

    fn scan(&self, _out: &mut Vec<Entry>) {}

    fn query(&self, input: &str, out: &mut Vec<Entry>) {
        let expr = input.trim_start_matches('=').trim();
        if expr.is_empty() {
            return;
        }
        if let Ok(value) = evalexpr::eval(expr) {
            let display = format_value(&value);
            out.push(Entry {
                name: display.clone(),
                description: Some(format!("= {expr}")),
                icon: Some("accessories-calculator".to_string()),
                kind: EntryKind::MathResult { value: display },
                priority: 0,
            });
        }
    }

    fn launch(&self, entry: &Entry) {
        if let EntryKind::MathResult { value } = &entry.kind {
            super::copy_to_clipboard(value, &self.clipboard);
        }
    }
}

fn format_value(value: &evalexpr::Value) -> String {
    match value {
        evalexpr::Value::Float(f) => {
            if f.fract() == 0.0 && f.abs() < 1e15 {
                format!("{}", *f as i64)
            } else {
                format!("{f}")
            }
        }
        evalexpr::Value::Int(i) => format!("{i}"),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(input: &str) -> Vec<Entry> {
        let mut out = Vec::new();
        MathPlugin::new(vec!["wl-copy".into()]).query(input, &mut out);
        out
    }

    #[test]
    fn evaluates_simple_expression() {
        let out = query("=1+2");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "3");
    }

    #[test]
    fn strips_leading_equals_and_whitespace() {
        let out = query("=  2 * 3  ");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "6");
    }

    #[test]
    fn whole_number_float_result_drops_decimal_point() {
        let out = query("=6/2");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "3");
    }

    #[test]
    fn fractional_float_result_keeps_decimals() {
        // A float operand forces float division (5/2 with two ints performs
        // integer division in evalexpr and would yield "2", not "2.5").
        let out = query("=5.0/2");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "2.5");
    }

    #[test]
    fn empty_expression_produces_no_entry() {
        assert!(query("=").is_empty());
        assert!(query("=   ").is_empty());
    }

    #[test]
    fn invalid_expression_produces_no_entry() {
        assert!(query("=this is not math").is_empty());
    }

    #[test]
    fn description_shows_the_normalized_expression() {
        let out = query("=1+2");
        assert_eq!(out[0].description.as_deref(), Some("= 1+2"));
    }

    #[test]
    fn entry_kind_carries_the_formatted_value_for_clipboard_copy() {
        let out = query("=1+2");
        match &out[0].kind {
            EntryKind::MathResult { value } => assert_eq!(value, "3"),
            _ => panic!("expected MathResult kind"),
        }
    }

    #[test]
    fn format_value_int() {
        assert_eq!(format_value(&evalexpr::Value::Int(42)), "42");
    }

    #[test]
    fn format_value_whole_float() {
        assert_eq!(format_value(&evalexpr::Value::Float(4.0)), "4");
    }

    #[test]
    fn format_value_fractional_float() {
        assert_eq!(format_value(&evalexpr::Value::Float(4.5)), "4.5");
    }

    #[test]
    fn format_value_boolean() {
        assert_eq!(format_value(&evalexpr::Value::Boolean(true)), "true");
    }
}
