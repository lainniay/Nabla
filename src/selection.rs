pub fn previous_wrapped(selected: usize, total: usize) -> usize {
    match total {
        0 => 0,
        _ if selected == 0 || selected >= total => total - 1,
        _ => selected - 1,
    }
}

pub fn next_wrapped(selected: usize, total: usize) -> usize {
    if total == 0 {
        0
    } else {
        selected.saturating_add(1) % total
    }
}

pub fn page_backward(selected: usize, rows: usize) -> usize {
    selected.saturating_sub(rows)
}

pub fn page_forward(selected: usize, total: usize, rows: usize) -> usize {
    selected.saturating_add(rows).min(total.saturating_sub(1))
}

pub fn centered_visible_start(total: usize, selected: usize, visible: usize) -> usize {
    if total == 0 || visible == 0 {
        return 0;
    }
    selected
        .min(total - 1)
        .saturating_sub(visible.saturating_sub(1) / 2)
        .min(total.saturating_sub(visible))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapped_navigation_handles_empty_and_out_of_range_selections() {
        assert_eq!(previous_wrapped(0, 0), 0);
        assert_eq!(next_wrapped(0, 0), 0);
        assert_eq!(previous_wrapped(0, 3), 2);
        assert_eq!(previous_wrapped(9, 3), 2);
        assert_eq!(next_wrapped(2, 3), 0);
    }

    #[test]
    fn paging_and_centering_are_saturating_and_bounded() {
        assert_eq!(page_backward(2, 8), 0);
        assert_eq!(page_forward(8, 10, 8), 9);
        assert_eq!(page_forward(0, 0, 8), 0);
        assert_eq!(centered_visible_start(30, 15, 24), 4);
        assert_eq!(centered_visible_start(2, 1, 24), 0);
    }
}
