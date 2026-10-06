//! ox.el's tables API for backends (org-export-table-*).

use super::Info;
use crate::org::element::{NodeId, Type};
use regex::Regex;
use std::sync::LazyLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
    Center,
}

/// A cell border (org-export-table-cell-borders).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Border {
    Top,
    Bottom,
    Above,
    Below,
    Left,
    Right,
}

static COOKIE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^<([lrc])?([0-9]+)?>$").unwrap());

impl Info {
    fn row_type(&self, row: NodeId) -> &str {
        self.tree.prop(row, "type").as_str().unwrap_or("")
    }

    /// The single string a cell holds (None if it holds objects or
    /// several pieces; Some("") when empty).
    fn cell_text(&self, cell: Option<NodeId>) -> Option<String> {
        let c = self.tree.contents(cell?);
        match c {
            [] => Some(String::new()),
            [x] => self.tree.plain_text(*x).map(str::to_owned),
            _ => None,
        }
    }

    fn cell_empty(&self, cell: Option<NodeId>) -> bool {
        cell.is_none_or(|c| self.tree.contents(c).is_empty())
    }

    /// org-export-table-has-special-column-p.
    pub fn table_has_special_column_p(&self, table: NodeId) -> bool {
        let mut special = false;
        for &row in self.tree.contents(table) {
            if self.row_type(row) != "standard" {
                continue;
            }
            let first = self.tree.contents(row).first().copied();
            if self.cell_empty(first) {
                continue;
            }
            match self.cell_text(first).as_deref() {
                Some("/" | "#" | "!" | "$" | "*" | "_" | "^") => special = true,
                _ => return false,
            }
        }
        special
    }

    /// org-export-table-has-header-p.
    pub fn table_has_header_p(&mut self, table: NodeId) -> bool {
        if let Some(&h) = self.caches.table_header.get(&table) {
            return h;
        }
        let mut group = 1;
        let mut flag = false;
        let r = self
            .map(
                &[table],
                Some(&[Type::TableRow]),
                &[],
                false,
                true,
                |i, row| {
                    if group > 1 {
                        return Some(());
                    }
                    let rule = i.row_type(row) == "rule";
                    if flag && rule {
                        group += 1;
                        flag = false;
                    } else if !flag && i.row_type(row) == "standard" {
                        flag = true;
                    }
                    None
                },
            )
            .is_empty();
        let r = !r;
        self.caches.table_header.insert(table, r);
        r
    }

    /// org-export-table-row-is-special-p.
    pub fn table_row_is_special_p(&self, row: NodeId) -> bool {
        if self.row_type(row) != "standard" {
            return false;
        }
        let first = self.tree.contents(row).first().copied();
        let ft = self.cell_text(first).filter(|_| !self.cell_empty(first));
        if ft.as_deref() == Some("/") {
            return true;
        }
        if let Some(t) = self.tree.parent(row)
            && self.table_has_special_column_p(t)
            && matches!(ft.as_deref(), Some("^" | "_" | "$" | "!"))
        {
            return true;
        }
        let mut cookie = false;
        for &cell in self.tree.contents(row) {
            if self.tree.contents(cell).is_empty() {
                continue;
            }
            match self.cell_text(Some(cell)) {
                Some(s) if regex_cookie(&s) => cookie = true,
                _ => return false,
            }
        }
        cookie
    }

    /// org-export-table-row-group.
    pub fn table_row_group(&mut self, row: NodeId) -> Option<usize> {
        if self.row_type(row) != "standard" {
            return None;
        }
        if let Some(&g) = self.caches.row_group.get(&row) {
            return g;
        }
        let table = self.tree.parent(row)?;
        let mut group = 0;
        let mut flag = false;
        let rows = self.map(
            &[table],
            Some(&[Type::TableRow]),
            &[],
            false,
            false,
            |i, r| {
                if i.row_type(r) == "rule" {
                    flag = false;
                    None
                } else {
                    if !flag {
                        group += 1;
                        flag = true;
                    }
                    Some((r, group))
                }
            },
        );
        for (r, g) in rows {
            self.caches.row_group.insert(r, Some(g));
        }
        self.caches.row_group.get(&row).copied().flatten()
    }

    /// The column of CELL in its row (special column included).
    pub fn cell_column(&self, cell: NodeId) -> usize {
        let row = self.tree.parent(cell).unwrap_or(cell);
        self.tree
            .contents(row)
            .iter()
            .position(|&c| c == cell)
            .unwrap_or(0)
    }

    /// org-export-table-cell-width: the width cookie of CELL's column.
    pub fn table_cell_width(&mut self, cell: NodeId) -> Option<usize> {
        let row = self.tree.parent(cell)?;
        let table = self.tree.parent(row)?;
        let columns = self.tree.contents(row).len();
        let column = self.cell_column(cell);
        let mut v = self
            .caches
            .cell_width
            .remove(&table)
            .unwrap_or_else(|| vec![None; columns]);
        if column >= v.len() {
            v.resize(column + 1, None);
        }
        let w = match v[column] {
            Some(w) => w,
            None => {
                let mut found = None;
                for &r in self.tree.contents(table) {
                    if !self.table_row_is_special_p(r) {
                        continue;
                    }
                    let c = self.tree.contents(r).get(column).copied();
                    if let Some(s) = self.cell_text(c)
                        && let Some(m) = COOKIE.captures(&s)
                        && let Some(n) = m.get(2)
                    {
                        found = n.as_str().parse().ok();
                        break;
                    }
                }
                v[column] = Some(found);
                found
            }
        };
        self.caches.cell_width.insert(table, v);
        w
    }

    /// org-export-table-cell-alignment.
    pub fn table_cell_alignment(&mut self, cell: NodeId) -> Align {
        let Some(row) = self.tree.parent(cell) else {
            return Align::Left;
        };
        let Some(table) = self.tree.parent(row) else {
            return Align::Left;
        };
        let columns = self.tree.contents(row).len();
        let column = self.cell_column(cell);
        let mut v = self
            .caches
            .cell_align
            .remove(&table)
            .unwrap_or_else(|| vec![None; columns]);
        if column >= v.len() {
            v.resize(column + 1, None);
        }
        if let Some(a) = v[column] {
            self.caches.cell_align.insert(table, v);
            return a;
        }
        let mut number = 0usize;
        let mut total = 0usize;
        let mut cookie: Option<char> = None;
        let mut prev_number = false;
        let num = crate::org::table::doc::number_re();
        for r in self.tree.contents(table).to_vec() {
            if self.table_row_is_special_p(r) {
                let c = self.tree.contents(r).get(column).copied();
                if let Some(s) = self.cell_text(c).filter(|s| !s.is_empty())
                    && let Some(m) = COOKIE.captures(&s)
                    && let Some(a) = m.get(1)
                {
                    cookie = a.as_str().chars().next();
                }
            } else if self.row_type(r) == "rule" {
            } else if cookie.is_none() {
                let kids = self
                    .tree
                    .contents(r)
                    .get(column)
                    .map(|&c| self.tree.contents(c).to_vec())
                    .unwrap_or_default();
                let value = self.data_seq(&kids);
                total += 1;
                if num.is_match(&value) || (value.is_empty() && prev_number) {
                    prev_number = true;
                    number += 1;
                } else {
                    prev_number = false;
                }
            }
        }
        let fraction = match crate::org::options::get("org-table-number-fraction") {
            Some(toml::Value::Float(f)) => f,
            Some(toml::Value::Integer(i)) => i as f64,
            _ => 0.5,
        };
        let a = match cookie {
            Some('l') => Align::Left,
            Some('r') => Align::Right,
            Some('c') => Align::Center,
            // 0/0 is NaN upstream: never >= the fraction.
            _ if total > 0 && number as f64 / total as f64 >= fraction => Align::Right,
            _ => Align::Left,
        };
        v[column] = Some(a);
        self.caches.cell_align.insert(table, v);
        a
    }

    /// org-export-table-cell-borders.
    pub fn table_cell_borders(&self, cell: NodeId) -> Vec<Border> {
        let mut borders = vec![];
        let Some(row) = self.tree.parent(cell) else {
            return borders;
        };
        let Some(table) = self.tree.lineage_of(cell, &[Type::Table], false) else {
            return borders;
        };
        let rows = self.tree.contents(table).to_vec();
        let i = rows.iter().position(|&r| r == row).unwrap_or(0);
        // Above.
        let mut rule = false;
        let mut done = false;
        for &r in rows[..i].iter().rev() {
            if self.row_type(r) == "rule" {
                rule = true;
            } else if !self.table_row_is_special_p(r) {
                if rule {
                    borders.insert(0, Border::Above);
                }
                done = true;
                break;
            }
        }
        if !done {
            if rule {
                borders.insert(0, Border::Above);
            }
            borders.insert(0, Border::Top);
        }
        // Below.
        let mut rule = false;
        let mut done = false;
        for &r in &rows[i + 1..] {
            if self.row_type(r) == "rule" {
                rule = true;
            } else if !self.table_row_is_special_p(r) {
                if rule {
                    borders.insert(0, Border::Below);
                }
                done = true;
                break;
            }
        }
        if !done {
            if rule {
                borders.insert(0, Border::Below);
            }
            borders.insert(0, Border::Bottom);
        }
        // Left and right: column groups.
        let column = self.cell_column(cell);
        for &r in rows.iter().rev() {
            if self.row_type(r) == "rule" {
                continue;
            }
            let first = self.tree.contents(r).first().copied();
            if self.cell_text(first).as_deref() != Some("/") || self.cell_empty(first) {
                continue;
            }
            let groups: Vec<Option<String>> = self
                .tree
                .contents(r)
                .iter()
                .map(|&c| {
                    if self.tree.contents(c).is_empty() {
                        return None;
                    }
                    self.cell_text(Some(c))
                        .filter(|s| matches!(s.as_str(), "<" | "<>" | ">"))
                })
                .collect();
            let g = |k: usize| groups.get(k).cloned().flatten();
            if (column > 0 && matches!(g(column - 1).as_deref(), Some(">" | "<>")))
                || matches!(g(column).as_deref(), Some("<" | "<>"))
            {
                borders.insert(0, Border::Left);
            }
            if (column + 1 != groups.len() && matches!(g(column + 1).as_deref(), Some("<" | "<>")))
                || matches!(g(column).as_deref(), Some(">" | "<>"))
            {
                borders.insert(0, Border::Right);
            }
            break;
        }
        borders
    }

    /// The first exportable cell of ROW.
    pub fn first_cell(&self, row: NodeId) -> Option<NodeId> {
        self.map(
            &[row],
            Some(&[Type::TableCell]),
            &[],
            false,
            true,
            |_, c| Some(c),
        )
        .into_iter()
        .next()
    }

    /// org-export-table-cell-starts-colgroup-p.
    pub fn table_cell_starts_colgroup_p(&self, cell: NodeId) -> bool {
        let row = self.tree.parent(cell).unwrap_or(cell);
        self.first_cell(row) == Some(cell) || self.table_cell_borders(cell).contains(&Border::Left)
    }

    /// org-export-table-cell-ends-colgroup-p.
    pub fn table_cell_ends_colgroup_p(&self, cell: NodeId) -> bool {
        let row = self.tree.parent(cell).unwrap_or(cell);
        self.tree.contents(row).last() == Some(&cell)
            || self.table_cell_borders(cell).contains(&Border::Right)
    }

    fn row_borders(&self, row: NodeId) -> Option<Vec<Border>> {
        if self.row_type(row) == "rule" || self.table_row_is_special_p(row) {
            return None;
        }
        let first = *self.tree.contents(row).first()?;
        Some(self.table_cell_borders(first))
    }

    /// org-export-table-row-starts-rowgroup-p.
    pub fn table_row_starts_rowgroup_p(&self, row: NodeId) -> bool {
        self.row_borders(row)
            .is_some_and(|b| b.contains(&Border::Top) || b.contains(&Border::Above))
    }

    /// org-export-table-row-ends-rowgroup-p.
    pub fn table_row_ends_rowgroup_p(&self, row: NodeId) -> bool {
        self.row_borders(row)
            .is_some_and(|b| b.contains(&Border::Bottom) || b.contains(&Border::Below))
    }

    /// org-export-table-row-in-header-p.
    pub fn table_row_in_header_p(&mut self, row: NodeId) -> bool {
        let Some(t) = self.tree.lineage_of(row, &[Type::Table], false) else {
            return false;
        };
        self.table_has_header_p(t) && self.table_row_group(row) == Some(1)
    }

    /// org-export-table-row-starts-header-p.
    pub fn table_row_starts_header_p(&mut self, row: NodeId) -> bool {
        self.table_row_in_header_p(row) && self.table_row_starts_rowgroup_p(row)
    }

    /// org-export-table-row-ends-header-p.
    pub fn table_row_ends_header_p(&mut self, row: NodeId) -> bool {
        self.table_row_in_header_p(row) && self.table_row_ends_rowgroup_p(row)
    }

    /// org-export-table-row-number.
    pub fn table_row_number(&mut self, row: NodeId) -> Option<usize> {
        if self.row_type(row) != "standard" {
            return None;
        }
        if let Some(&n) = self.caches.row_number.get(&row) {
            return n;
        }
        let table = self.tree.lineage_of(row, &[Type::Table], false)?;
        let mut n = 0;
        let rows = self.map(
            &[table],
            Some(&[Type::TableRow]),
            &[],
            false,
            false,
            |i, r| {
                (i.row_type(r) == "standard").then(|| {
                    n += 1;
                    (r, n - 1)
                })
            },
        );
        for (r, k) in rows {
            self.caches.row_number.insert(r, Some(k));
        }
        self.caches.row_number.get(&row).copied().flatten()
    }

    /// org-export-table-dimensions: (rows, columns).
    pub fn table_dimensions(&self, table: NodeId) -> (usize, usize) {
        let rows: Vec<NodeId> = self.map(
            &[table],
            Some(&[Type::TableRow]),
            &[],
            false,
            false,
            |i, r| (i.row_type(r) == "standard").then_some(r),
        );
        let cols = rows.first().map_or(0, |&r| {
            self.map(&[r], Some(&[Type::TableCell]), &[], false, false, |_, c| {
                Some(c)
            })
            .len()
        });
        (rows.len(), cols)
    }

    /// org-export-table-cell-address: (row, column) among exportable cells.
    pub fn table_cell_address(&mut self, cell: NodeId) -> Option<(usize, usize)> {
        let row = self.tree.parent(cell)?;
        let r = self.table_row_number(row)?;
        let cells = self.map(
            &[row],
            Some(&[Type::TableCell]),
            &[],
            false,
            false,
            |_, c| Some(c),
        );
        Some((r, cells.iter().position(|&c| c == cell)?))
    }

    /// org-export-get-table-cell-at.
    pub fn table_cell_at(&self, address: (usize, usize), table: NodeId) -> Option<NodeId> {
        let rows: Vec<NodeId> = self.map(
            &[table],
            Some(&[Type::TableRow]),
            &[],
            false,
            false,
            |i, r| (i.row_type(r) != "rule").then_some(r),
        );
        let row = *rows.get(address.0)?;
        self.map(
            &[row],
            Some(&[Type::TableCell]),
            &[],
            false,
            false,
            |_, c| Some(c),
        )
        .get(address.1)
        .copied()
    }
}

fn regex_cookie(s: &str) -> bool {
    static R: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^<[lrc]?([0-9]+)?>$").unwrap());
    R.is_match(s)
}
