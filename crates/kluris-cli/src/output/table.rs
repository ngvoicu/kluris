//! A plain-text table for human output (v2's Rich tables, without the boxes).

/// Render `title`, a header row, a rule and the rows, columns padded to the
/// widest cell (in characters).
pub fn render(title: &str, headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let line = |cells: &mut dyn Iterator<Item = &str>| {
        let padded: Vec<String> = cells
            .zip(&widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect();
        format!("{}\n", padded.join("  ").trim_end())
    };
    let mut out = format!("{title}\n");
    out.push_str(&line(&mut headers.iter().copied()));
    out.push_str(&line(
        &mut widths
            .iter()
            .map(|w| "-".repeat(*w))
            .collect::<Vec<_>>()
            .iter()
            .map(String::as_str),
    ));
    for row in rows {
        out.push_str(&line(&mut row.iter().map(String::as_str)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pads_columns_to_the_widest_cell() {
        let rows = vec![vec!["a".to_string(), "Grüße".to_string()]];
        assert_eq!(
            render("T", &["Name", "X"], &rows),
            "T\nName  X\n----  -----\na     Grüße\n"
        );
    }
}
