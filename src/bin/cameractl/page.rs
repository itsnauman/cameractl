//! Page windows for list commands.

use crate::error::CliError;

#[derive(Debug, Clone, Copy)]
pub struct Page {
    pub number: usize,
    pub all: bool,
    pub size: Option<usize>,
}

#[derive(Debug, Clone, Copy)]
pub struct Window {
    pub start: usize,
    pub end: usize,
    pub number: usize,
    pub page_size: usize,
    pub pages: usize,
}

pub fn resolve(
    page: Option<usize>,
    page_size: Option<usize>,
    page_all: bool,
    tty: bool,
) -> Result<Page, CliError> {
    if page_all && (page.is_some() || page_size.is_some()) {
        return Err(CliError::usage(
            "--page-all cannot be combined with --page or --page-size",
            "Drop --page-all, or drop --page and --page-size.",
        ));
    }
    if page == Some(0) {
        return Err(CliError::usage("--page starts at 1", "Pass --page 1."));
    }
    if page_size == Some(0) {
        return Err(CliError::usage(
            "--page-size must be at least 1",
            "Pass --page-size 20 or --page-all.",
        ));
    }
    if let Some(size) = page_size {
        if size > 10_000 {
            return Err(CliError::usage(
                "--page-size cannot exceed 10000",
                "Pass --page-all to return every item.",
            ));
        }
    }
    if page_all || (!tty && page.is_none() && page_size.is_none()) {
        return Ok(Page {
            number: 1,
            all: true,
            size: None,
        });
    }
    Ok(Page {
        number: page.unwrap_or(1),
        all: false,
        size: Some(page_size.unwrap_or(20)),
    })
}

impl Page {
    pub fn window(self, total: usize) -> Window {
        if self.all {
            return Window {
                start: 0,
                end: total,
                number: 1,
                page_size: total,
                pages: if total == 0 { 0 } else { 1 },
            };
        }
        let size = self.size.unwrap_or(20);
        let pages = if total == 0 { 0 } else { total.div_ceil(size) };
        let start = self.number.saturating_sub(1).saturating_mul(size);
        let start = start.min(total);
        let end = start.saturating_add(size).min(total);
        Window {
            start,
            end,
            number: self.number,
            page_size: size,
            pages,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pipe_defaults_to_every_item_and_a_terminal_pages() {
        assert!(resolve(None, None, false, false).unwrap().all);
        let page = resolve(None, None, false, true).unwrap();
        assert_eq!(page.size, Some(20));
        let window = page.window(45);
        assert_eq!((window.start, window.end, window.pages), (0, 20, 3));
    }

    #[test]
    fn page_zero_is_usage() {
        assert_eq!(resolve(Some(0), None, false, false).unwrap_err().exit, 2);
    }
}
