use super::{Entry, EntryKind, Plugin};

pub struct MathPlugin;

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
            if let Err(e) = std::process::Command::new("wl-copy").arg(value).spawn() {
                tracing::error!("Failed to copy math result to clipboard: {e}");
            }
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
