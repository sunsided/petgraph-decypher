//! Query results.

use crate::value::Value;

/// The rows a query returns, with its column names.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct QueryResult {
    columns: Vec<String>,
    rows: Vec<Vec<Value>>,
}

/// One row of a [`QueryResult`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Row<'a> {
    columns: &'a [String],
    values: &'a [Value],
}

impl QueryResult {
    pub(crate) fn new(columns: Vec<String>, rows: Vec<Vec<Value>>) -> Self {
        Self { columns, rows }
    }

    /// Column names in `RETURN` order.
    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    /// The number of rows.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether there are no rows.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The rows in order.
    pub fn rows(&self) -> impl ExactSizeIterator<Item = Row<'_>> {
        self.rows.iter().map(|values| Row {
            columns: &self.columns,
            values,
        })
    }

    /// The values of column `name`, one per row; empty if there is no such
    /// column.
    pub fn column(&self, name: &str) -> impl Iterator<Item = &Value> {
        let index = self.columns.iter().position(|c| c == name);
        self.rows.iter().filter_map(move |row| row.get(index?))
    }

    /// The raw rows, each with one value per column.
    pub fn into_rows(self) -> Vec<Vec<Value>> {
        self.rows
    }
}

impl<'a> Row<'a> {
    /// The value of column `name`.
    pub fn get(&self, name: &str) -> Option<&'a Value> {
        let index = self.columns.iter().position(|c| c == name)?;
        self.values.get(index)
    }

    /// The values in column order.
    pub fn values(&self) -> &'a [Value] {
        self.values
    }

    /// `(column, value)` pairs in column order.
    pub fn iter(&self) -> impl Iterator<Item = (&'a str, &'a Value)> {
        self.columns.iter().map(String::as_str).zip(self.values)
    }
}
