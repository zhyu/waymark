use std::cmp::Ordering;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::Result;
use crate::db::{Entry, ScoredEntry};

const FZF_ARGS: &[&str] = &["-1", "-0", "--no-sort", "+m"];

pub fn score_entries(entries: Vec<Entry>, tokens: &[String], now: i64) -> Vec<ScoredEntry> {
    let mut scored = entries
        .into_iter()
        .filter_map(|entry| {
            let match_score = match_score(&entry.path, tokens)?;
            let score = match_score * frecency_score(entry.rank, entry.last_accessed, now);
            Some(ScoredEntry { entry, score })
        })
        .collect::<Vec<_>>();

    scored.sort_by(compare_scored);
    scored
}

pub fn select_interactive(results: &[ScoredEntry]) -> Result<Option<String>> {
    let Some(first) = results.first() else {
        return Ok(None);
    };

    let mut child = match Command::new("fzf")
        .args(FZF_ARGS)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Some(first.entry.path.clone()));
        }
        Err(error) => return Err(error.into()),
    };

    {
        let mut stdin = child.stdin.take().expect("fzf stdin was piped");
        for result in results {
            writeln!(stdin, "{}", result.entry.path)?;
        }
    }

    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Ok(None);
    }

    let selected = String::from_utf8_lossy(&output.stdout);
    let selected = selected.trim_end_matches(['\r', '\n']);
    if selected.is_empty() {
        Ok(None)
    } else {
        Ok(Some(selected.to_string()))
    }
}

fn compare_scored(a: &ScoredEntry, b: &ScoredEntry) -> Ordering {
    b.score
        .total_cmp(&a.score)
        .then_with(|| b.entry.last_accessed.cmp(&a.entry.last_accessed))
        .then_with(|| b.entry.access_count.cmp(&a.entry.access_count))
        .then_with(|| a.entry.path.len().cmp(&b.entry.path.len()))
        .then_with(|| a.entry.path.cmp(&b.entry.path))
}

fn frecency_score(rank: f64, last_accessed: i64, now: i64) -> f64 {
    rank.max(0.001) * recency_multiplier(now.saturating_sub(last_accessed))
}

fn recency_multiplier(age_seconds: i64) -> f64 {
    match age_seconds {
        age if age <= 60 * 60 => 6.0,
        age if age <= 24 * 60 * 60 => 4.0,
        age if age <= 7 * 24 * 60 * 60 => 2.0,
        _ => 1.0,
    }
}

fn match_score(path: &str, tokens: &[String]) -> Option<f64> {
    let tokens = tokens
        .iter()
        .map(|token| token.trim())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        return Some(1.0);
    }

    let mut cursor = 0;
    let mut score = 0.0;
    for token in tokens {
        let matched = find_case_sensitive(path, token, cursor)
            .map(|position| Match {
                position,
                end: position + token.len(),
                base: 100.0,
            })
            .or_else(|| {
                find_case_insensitive(path, token, cursor).map(|(position, end)| Match {
                    position,
                    end,
                    base: 70.0,
                })
            })
            .or_else(|| {
                find_fuzzy(path, token, cursor).map(|(position, end)| Match {
                    position,
                    end,
                    base: 25.0,
                })
            })?;

        score +=
            matched.base * boundary_boost(path, matched.position) * basename_boost(path, token);
        cursor = matched.end;
    }

    Some(score)
}

#[derive(Clone, Copy, Debug)]
struct Match {
    position: usize,
    end: usize,
    base: f64,
}

fn find_case_sensitive(path: &str, token: &str, cursor: usize) -> Option<usize> {
    path.get(cursor..)?
        .find(token)
        .map(|offset| cursor + offset)
}

fn find_case_insensitive(path: &str, token: &str, cursor: usize) -> Option<(usize, usize)> {
    let lower_path = path.to_ascii_lowercase();
    let lower_token = token.to_ascii_lowercase();
    lower_path.get(cursor..)?.find(&lower_token).map(|offset| {
        let position = cursor + offset;
        (position, position + token.len())
    })
}

fn find_fuzzy(path: &str, token: &str, cursor: usize) -> Option<(usize, usize)> {
    let mut token_chars = token.chars().map(|ch| ch.to_ascii_lowercase());
    let mut wanted = token_chars.next()?;
    let mut start = None;

    for (offset, ch) in path.get(cursor..)?.char_indices() {
        if ch.to_ascii_lowercase() == wanted {
            let position = cursor + offset;
            start.get_or_insert(position);
            let end = position + ch.len_utf8();
            match token_chars.next() {
                Some(next) => wanted = next,
                None => return Some((start.unwrap_or(position), end)),
            }
        }
    }
    None
}

fn boundary_boost(path: &str, position: usize) -> f64 {
    if position == 0 {
        return 1.35;
    }
    match path[..position].chars().next_back() {
        Some('/') | Some('\\') | Some('-') | Some('_') | Some('.') | Some(' ') => 1.35,
        _ => 1.0,
    }
}

fn basename_boost(path: &str, token: &str) -> f64 {
    let basename = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path);

    if basename == token {
        3.0
    } else if basename.eq_ignore_ascii_case(token) {
        2.5
    } else if basename.contains(token) {
        1.6
    } else if basename
        .to_ascii_lowercase()
        .contains(&token.to_ascii_lowercase())
    {
        1.3
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_basename_beats_parent_path_match() {
        let now = 10_000;
        let results = score_entries(
            vec![
                entry("/tmp/config/project/readme.md", 2.0, now, 10),
                entry("/tmp/project/config", 1.0, now, 1),
            ],
            &["config".to_string()],
            now,
        );

        assert_eq!(results[0].entry.path, "/tmp/project/config");
    }

    #[test]
    fn rank_and_recency_affect_ordering() {
        let now = 10_000;
        let results = score_entries(
            vec![
                entry("/tmp/project/old-config", 20.0, now - 10 * 24 * 60 * 60, 20),
                entry("/tmp/project/new-config", 6.0, now - 60, 6),
            ],
            &["config".to_string()],
            now,
        );

        assert_eq!(results[0].entry.path, "/tmp/project/new-config");
    }

    #[test]
    fn multi_token_matches_in_order() {
        let now = 10_000;
        let results = score_entries(
            vec![
                entry("/tmp/dotfiles/zsh/zshrc", 1.0, now, 1),
                entry("/tmp/zshrc/dotfiles/zsh", 1.0, now, 1),
            ],
            &["dot".to_string(), "zshrc".to_string()],
            now,
        );

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.path, "/tmp/dotfiles/zsh/zshrc");
    }

    #[test]
    fn case_insensitive_fallback_matches_paths() {
        let now = 10_000;
        let results = score_entries(
            vec![entry("/tmp/project/Config.toml", 1.0, now, 1)],
            &["config".to_string()],
            now,
        );

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.path, "/tmp/project/Config.toml");
    }

    #[test]
    fn fuzzy_fallback_matches_ordered_characters() {
        let now = 10_000;
        let results = score_entries(
            vec![
                entry("/tmp/project/zshrc", 1.0, now, 1),
                entry("/tmp/project/rcshz", 1.0, now, 1),
            ],
            &["zrc".to_string()],
            now,
        );

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.path, "/tmp/project/zshrc");
    }

    #[test]
    fn path_segment_boundaries_boost_matches() {
        let now = 10_000;
        let results = score_entries(
            vec![
                entry("/tmp/project/myconfig", 1.0, now, 1),
                entry("/tmp/project/my/config", 1.0, now, 1),
            ],
            &["config".to_string()],
            now,
        );

        assert_eq!(results[0].entry.path, "/tmp/project/my/config");
    }

    #[test]
    fn empty_query_returns_ranked_entries() {
        let now = 10_000;
        let results = score_entries(
            vec![
                entry("/tmp/project/low", 1.0, now, 1),
                entry("/tmp/project/high", 3.0, now, 3),
            ],
            &[],
            now,
        );

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].entry.path, "/tmp/project/high");
    }

    #[test]
    fn tie_breakers_are_deterministic() {
        let now = 10_000;
        let results = score_entries(
            vec![
                entry("/tmp/b/config", 1.0, now, 1),
                entry("/tmp/a/config", 1.0, now, 1),
            ],
            &["config".to_string()],
            now,
        );

        assert_eq!(results[0].entry.path, "/tmp/a/config");
    }

    #[test]
    fn interactive_fzf_preserves_waymark_order_and_single_selection() {
        assert_eq!(FZF_ARGS, ["-1", "-0", "--no-sort", "+m"]);
    }

    fn entry(path: &str, rank: f64, last_accessed: i64, access_count: i64) -> Entry {
        Entry {
            path: path.to_string(),
            kind: "file".to_string(),
            rank,
            access_count,
            first_seen: 0,
            last_accessed,
            last_seen: last_accessed,
            source: "test".to_string(),
        }
    }
}
