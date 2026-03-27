use ratatui::style::Style;
use ratatui::text::{Line, Span};
use serde_json::Value;

use crate::theme::Theme;

const INDENT: &str = "  ";

#[derive(Clone, Copy)]
struct JsonStyles {
    key: Style,
    string: Style,
    number: Style,
    bool_null: Style,
    punct: Style,
}

impl From<&Theme> for JsonStyles {
    fn from(theme: &Theme) -> Self {
        Self {
            key: theme.json_key,
            string: theme.json_string,
            number: theme.json_number,
            bool_null: theme.json_bool_null,
            punct: theme.json_punct,
        }
    }
}

pub fn json_to_lines(value: &Value, theme: &Theme) -> Vec<Line<'static>> {
    let styles = JsonStyles::from(theme);
    let mut lines = Vec::new();
    render_value(value, 0, false, &styles, &mut lines);
    lines
}

fn render_value(
    value: &Value,
    depth: usize,
    trailing_comma: bool,
    s: &JsonStyles,
    lines: &mut Vec<Line<'static>>,
) {
    let comma = if trailing_comma { "," } else { "" };
    let prefix = INDENT.repeat(depth);

    match value {
        Value::Null => {
            lines.push(Line::from(vec![
                Span::raw(prefix),
                Span::styled("null", s.bool_null),
                Span::styled(comma, s.punct),
            ]));
        }
        Value::Bool(b) => {
            lines.push(Line::from(vec![
                Span::raw(prefix),
                Span::styled(b.to_string(), s.bool_null),
                Span::styled(comma, s.punct),
            ]));
        }
        Value::Number(n) => {
            lines.push(Line::from(vec![
                Span::raw(prefix),
                Span::styled(n.to_string(), s.number),
                Span::styled(comma, s.punct),
            ]));
        }
        Value::String(st) => {
            lines.push(Line::from(vec![
                Span::raw(prefix),
                Span::styled(format!("\"{}\"", escape_json_string(st)), s.string),
                Span::styled(comma, s.punct),
            ]));
        }
        Value::Array(arr) => {
            if arr.is_empty() {
                lines.push(Line::from(vec![
                    Span::raw(prefix),
                    Span::styled(format!("[]{comma}"), s.punct),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::raw(prefix.clone()),
                    Span::styled("[", s.punct),
                ]));
                for (i, item) in arr.iter().enumerate() {
                    render_value(item, depth + 1, i + 1 < arr.len(), s, lines);
                }
                lines.push(Line::from(vec![
                    Span::raw(prefix),
                    Span::styled(format!("]{comma}"), s.punct),
                ]));
            }
        }
        Value::Object(obj) => {
            if obj.is_empty() {
                lines.push(Line::from(vec![
                    Span::raw(prefix),
                    Span::styled(format!("{{}}{comma}"), s.punct),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::raw(prefix.clone()),
                    Span::styled("{", s.punct),
                ]));
                let mut keys: Vec<&String> = obj.keys().collect();
                keys.sort();
                let count = keys.len();
                for (i, key) in keys.into_iter().enumerate() {
                    let val = &obj[key];
                    let has_comma = i + 1 < count;
                    let child_prefix = INDENT.repeat(depth + 1);
                    if matches!(val, Value::Object(_) | Value::Array(_)) && !is_empty_container(val)
                    {
                        lines.push(Line::from(vec![
                            Span::raw(child_prefix),
                            Span::styled(format!("\"{}\"", escape_json_string(key)), s.key),
                            Span::styled(": ", s.punct),
                        ]));
                        render_value_inline_open(val, depth + 1, has_comma, s, lines);
                    } else {
                        let mut spans = vec![
                            Span::raw(child_prefix),
                            Span::styled(format!("\"{}\"", escape_json_string(key)), s.key),
                            Span::styled(": ", s.punct),
                        ];
                        append_inline_value(val, has_comma, s, &mut spans);
                        lines.push(Line::from(spans));
                    }
                }
                lines.push(Line::from(vec![
                    Span::raw(prefix),
                    Span::styled(format!("}}{comma}"), s.punct),
                ]));
            }
        }
    }
}

fn render_value_inline_open(
    value: &Value,
    depth: usize,
    trailing_comma: bool,
    s: &JsonStyles,
    lines: &mut Vec<Line<'static>>,
) {
    let comma = if trailing_comma { "," } else { "" };
    let prefix = INDENT.repeat(depth);
    match value {
        Value::Array(arr) => {
            if let Some(last_line) = lines.last_mut() {
                last_line.spans.push(Span::styled("[", s.punct));
            }
            for (i, item) in arr.iter().enumerate() {
                render_value(item, depth + 1, i + 1 < arr.len(), s, lines);
            }
            lines.push(Line::from(vec![
                Span::raw(prefix),
                Span::styled(format!("]{comma}"), s.punct),
            ]));
        }
        Value::Object(obj) => {
            if let Some(last_line) = lines.last_mut() {
                last_line.spans.push(Span::styled("{", s.punct));
            }
            let mut keys: Vec<&String> = obj.keys().collect();
            keys.sort();
            let count = keys.len();
            for (i, key) in keys.into_iter().enumerate() {
                let val = &obj[key];
                let has_comma = i + 1 < count;
                let child_prefix = INDENT.repeat(depth + 1);
                if matches!(val, Value::Object(_) | Value::Array(_)) && !is_empty_container(val) {
                    lines.push(Line::from(vec![
                        Span::raw(child_prefix),
                        Span::styled(format!("\"{}\"", escape_json_string(key)), s.key),
                        Span::styled(": ", s.punct),
                    ]));
                    render_value_inline_open(val, depth + 1, has_comma, s, lines);
                } else {
                    let mut spans = vec![
                        Span::raw(child_prefix),
                        Span::styled(format!("\"{}\"", escape_json_string(key)), s.key),
                        Span::styled(": ", s.punct),
                    ];
                    append_inline_value(val, has_comma, s, &mut spans);
                    lines.push(Line::from(spans));
                }
            }
            lines.push(Line::from(vec![
                Span::raw(prefix),
                Span::styled(format!("}}{comma}"), s.punct),
            ]));
        }
        _ => {
            render_value(value, depth, trailing_comma, s, lines);
        }
    }
}

fn append_inline_value(
    value: &Value,
    trailing_comma: bool,
    s: &JsonStyles,
    spans: &mut Vec<Span<'static>>,
) {
    let comma = if trailing_comma { "," } else { "" };
    match value {
        Value::Null => {
            spans.push(Span::styled("null", s.bool_null));
            spans.push(Span::styled(comma, s.punct));
        }
        Value::Bool(b) => {
            spans.push(Span::styled(b.to_string(), s.bool_null));
            spans.push(Span::styled(comma, s.punct));
        }
        Value::Number(n) => {
            spans.push(Span::styled(n.to_string(), s.number));
            spans.push(Span::styled(comma, s.punct));
        }
        Value::String(st) => {
            spans.push(Span::styled(
                format!("\"{}\"", escape_json_string(st)),
                s.string,
            ));
            spans.push(Span::styled(comma, s.punct));
        }
        Value::Array(arr) if arr.is_empty() => {
            spans.push(Span::styled(format!("[]{comma}"), s.punct));
        }
        Value::Object(obj) if obj.is_empty() => {
            spans.push(Span::styled(format!("{{}}{comma}"), s.punct));
        }
        _ => {
            spans.push(Span::raw("..."));
        }
    }
}

fn is_empty_container(value: &Value) -> bool {
    matches!(value, Value::Array(a) if a.is_empty())
        || matches!(value, Value::Object(o) if o.is_empty())
}

fn escape_json_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}
