//! Fuzzy matching and search ranking algorithms for the launcher.

/// Computes a fuzzy match score between a user query and a target string.
///
/// Higher scores represent better matches. Returns `None` if the target
/// does not contain all query characters in sequence.
/// Optimized to stream characters without heap allocating `Vec<char>`.
pub fn fuzzy_match(query: &str, target: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }

    let mut q_chars = query.chars();
    let mut current_q = match q_chars.next() {
        Some(c) => c,
        None => return Some(0),
    };

    let mut score = 0;
    let mut consecutive = 0;
    let mut prev_c = '\0';

    for (t_idx, t_c) in target.chars().enumerate() {
        if t_c.eq_ignore_ascii_case(&current_q) {
            score += 10;
            score += consecutive * 5;
            if t_idx == 0 {
                score += 15; // Starting character bonus
            } else if prev_c == '-' || prev_c == '_' || prev_c == '.' || prev_c == ' ' {
                score += 10; // Word boundary bonus (e.g. "fc" matching "firewall-cmd")
            }
            consecutive += 1;
            match q_chars.next() {
                Some(next_q) => current_q = next_q,
                None => return Some(score),
            }
        } else {
            consecutive = 0;
            score -= 1; // Gap penalty
        }
        prev_c = t_c;
    }

    None
}

#[inline]
fn starts_with_ignore_case(haystack: &str, needle: &str) -> bool {
    if needle.len() > haystack.len() {
        return false;
    }
    haystack
        .chars()
        .zip(needle.chars())
        .all(|(h, n)| h.eq_ignore_ascii_case(&n))
}

#[inline]
fn find_ignore_case(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    let needle_chars: Vec<char> = needle.chars().collect();
    let haystack_chars: Vec<char> = haystack.chars().collect();
    if needle_chars.len() > haystack_chars.len() {
        return None;
    }
    for i in 0..=haystack_chars.len() - needle_chars.len() {
        if haystack_chars[i..i + needle_chars.len()]
            .iter()
            .zip(needle_chars.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
        {
            return Some(i);
        }
    }
    None
}

fn score_item(item: &str, query: &str) -> Option<i32> {
    if query.is_empty() {
        return None;
    }

    if item.eq_ignore_ascii_case(query) {
        // Exact match
        Some(5000)
    } else if starts_with_ignore_case(item, query) {
        // High priority for prefix matches
        Some(3000 - (item.len() as i32 * 2))
    } else if let Some(fuzzy_score) = fuzzy_match(query, item) {
        let first_char = query.chars().next();
        let starts_with_first = first_char.is_some_and(|fc| {
            item.chars().next().is_some_and(|ic| ic.eq_ignore_ascii_case(&fc))
        });
        if starts_with_first {
            Some(1500 + fuzzy_score * 10 - (item.len() as i32))
        } else if let Some(pos) = find_ignore_case(item, query) {
            Some(500 - (pos as i32 * 5) - (item.len() as i32))
        } else {
            Some(fuzzy_score * 5 - (item.len() as i32))
        }
    } else {
        find_ignore_case(item, query).map(|pos| 500 - (pos as i32 * 5) - (item.len() as i32))
    }
}

/// Filters and ranks cached executable names against the user query.
///
/// Matches against the full query (allowing multi-word searches like "Visual Studio Code"),
/// as well as the initial binary name if CLI arguments are provided (e.g. `firefox --private-window`).
pub fn filter_executables(cache: &[String], query: &str, max_results: usize) -> Vec<String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return cache.iter().take(max_results).cloned().collect();
    }

    let binary_query = trimmed.split_whitespace().next().unwrap_or("").trim();
    let has_args = binary_query.len() < trimmed.len();

    let mut scored: Vec<(&String, i32)> = Vec::new();

    for item in cache {
        let full_score = score_item(item, trimmed);
        let score = if has_args {
            let bin_score = score_item(item, binary_query);
            match (full_score, bin_score) {
                (Some(s1), Some(s2)) => Some(s1.max(s2)),
                (Some(s1), None) => Some(s1),
                (None, Some(s2)) => Some(s2),
                (None, None) => None,
            }
        } else {
            full_score
        };

        if let Some(s) = score {
            scored.push((item, s));
        }
    }

    scored.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| a.0.len().cmp(&b.0.len()))
            .then_with(|| a.0.cmp(b.0))
    });
    scored.into_iter().take(max_results).map(|(s, _)| s.clone()).collect()
}
