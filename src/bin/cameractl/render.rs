//! stdout renderers. JSON is the contract; table, csv, and ndjson are views.

use serde_json::Value;

use crate::cli::Format;
use crate::error::CliError;
use crate::present::Column;

#[derive(Debug)]
pub struct ListOut {
    pub items: Vec<Value>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
    pub pages: usize,
    pub columns: &'static [Column],
}

impl ListOut {
    pub fn full(items: Vec<Value>, columns: &'static [Column]) -> Self {
        let total = items.len();
        Self {
            items,
            total,
            page: 1,
            page_size: total,
            pages: if total == 0 { 0 } else { 1 },
            columns,
        }
    }

    pub fn page(
        items: Vec<Value>,
        total: usize,
        window: crate::page::Window,
        columns: &'static [Column],
    ) -> Self {
        Self {
            items,
            total,
            page: window.number,
            page_size: window.page_size,
            pages: window.pages,
            columns,
        }
    }
}

pub enum Document {
    One(Value),
    Many(ListOut),
    Text(String),
}

pub fn errors_are_json(format: Format) -> bool {
    !matches!(format, Format::Table)
}

pub fn json_error(error: &CliError) -> String {
    format!("{}\n", compact(&error.json()))
}

pub fn render(document: &Document, format: Format, color: bool) -> String {
    match document {
        Document::Text(text) => newline(text),
        Document::One(value) => match format {
            Format::Json | Format::Ndjson => format!("{}\n", compact(value)),
            Format::Csv => csv(
                &["field".to_owned(), "value".to_owned()],
                &object_rows(value),
            ),
            Format::Table => table(
                &["field".to_owned(), "value".to_owned()],
                &object_rows(value),
                color,
            ),
        },
        Document::Many(list) => match format {
            Format::Json => format!(
                "{}\n",
                compact(&serde_json::json!({
                    "items": list.items,
                    "total": list.total,
                    "page": list.page,
                    "page_size": list.page_size,
                    "pages": list.pages,
                }))
            ),
            Format::Ndjson => {
                let mut out = String::new();
                for item in &list.items {
                    out.push_str(&compact(item));
                    out.push('\n');
                }
                out
            }
            Format::Csv => {
                let headers: Vec<String> = list
                    .columns
                    .iter()
                    .map(|column| column.header.to_owned())
                    .collect();
                csv(&headers, &list_rows(list))
            }
            Format::Table => {
                let headers: Vec<String> = list
                    .columns
                    .iter()
                    .map(|column| column.header.to_owned())
                    .collect();
                table(&headers, &list_rows(list), color)
            }
        },
    }
}

fn object_rows(value: &Value) -> Vec<Vec<String>> {
    let Some(map) = value.as_object() else {
        return vec![vec!["value".to_owned(), cell(value, false)]];
    };
    map.iter()
        .map(|(key, item)| vec![key.clone(), cell(item, false)])
        .collect()
}

fn list_rows(list: &ListOut) -> Vec<Vec<String>> {
    list.items
        .iter()
        .map(|item| {
            list.columns
                .iter()
                .map(|column| cell(item.get(column.key).unwrap_or(&Value::Null), false))
                .collect()
        })
        .collect()
}

fn cell(value: &Value, truncate: bool) -> String {
    let text = match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(value) => value.to_string(),
        other => compact(other),
    };
    if truncate && text.chars().count() > 40 {
        let mut shortened: String = text.chars().take(39).collect();
        shortened.push('…');
        shortened
    } else {
        text
    }
}

fn table(headers: &[String], rows: &[Vec<String>], color: bool) -> String {
    if headers.is_empty() {
        return String::new();
    }
    let shown: Vec<Vec<String>> = rows
        .iter()
        .map(|row| row.iter().map(|cell| truncate_cell(cell)).collect())
        .collect();
    let mut widths: Vec<usize> = headers
        .iter()
        .map(|header| header.chars().count())
        .collect();
    for row in &shown {
        for (index, cell) in row.iter().enumerate() {
            if let Some(width) = widths.get_mut(index) {
                *width = (*width).max(cell.chars().count());
            }
        }
    }
    let mut out = String::new();
    push_row(&mut out, headers, &widths, color);
    for row in &shown {
        push_row(&mut out, row, &widths, false);
    }
    out
}

fn truncate_cell(text: &str) -> String {
    cell(&Value::String(text.to_owned()), true)
}

fn push_row(out: &mut String, cells: &[String], widths: &[usize], bold: bool) {
    if bold {
        out.push_str("\u{1b}[1m");
    }
    for (index, cell) in cells.iter().enumerate() {
        if index > 0 {
            out.push_str("  ");
        }
        let width = widths.get(index).copied().unwrap_or(0);
        let pad = width.saturating_sub(cell.chars().count());
        out.push_str(cell);
        out.push_str(&" ".repeat(pad));
    }
    if bold {
        out.push_str("\u{1b}[0m");
    }
    out.push('\n');
}

fn csv(headers: &[String], rows: &[Vec<String>]) -> String {
    let mut out = headers
        .iter()
        .map(|header| csv_escape(header))
        .collect::<Vec<_>>()
        .join(",");
    out.push('\n');
    for row in rows {
        out.push_str(
            &row.iter()
                .map(|cell| csv_escape(cell))
                .collect::<Vec<_>>()
                .join(","),
        );
        out.push('\n');
    }
    out
}

fn csv_escape(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_owned()
    }
}

fn compact(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "{\"error\":\"encode\"}".to_owned())
}

fn newline(text: &str) -> String {
    if text.ends_with('\n') {
        text.to_owned()
    } else {
        format!("{text}\n")
    }
}
