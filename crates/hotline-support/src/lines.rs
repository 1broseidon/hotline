//! How many lines an edit added and removed, as a diff counts them: the
//! fewest lines taken out and put in that turn the old text into the new
//! (Myers' shortest edit script). A moved line is one out and one in, as it
//! is in any diff a person reads.

/// The most lines either side may differ by before the count is given up:
/// past it the walk costs more than a glance at an edit is worth, and no
/// count is better than a wrong one.
const MOST_CHANGED: usize = 4_000;

/// Lines added and removed from `before` to `after`, or `None` when the
/// edit is too large to count.
pub fn changed_lines(before: &str, after: &str) -> Option<(u32, u32)> {
    let old: Vec<&str> = before.lines().collect();
    let new: Vec<&str> = after.lines().collect();
    // What the two share at either end is unchanged and needs no walk.
    let head = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let tail = old[head..]
        .iter()
        .rev()
        .zip(new[head..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let old = &old[head..old.len() - tail];
    let new = &new[head..new.len() - tail];
    let distance = edit_distance(old, new)?;
    // Every edit is a line in or a line out, and the difference in length is
    // all lines in or all lines out.
    let added = (distance + new.len() - old.len()) / 2;
    let removed = distance - added;
    Some((u32::try_from(added).ok()?, u32::try_from(removed).ok()?))
}

/// The length of the shortest edit script (inserts and deletes) from `a`
/// to `b`, walked along Myers' furthest-reaching diagonals.
fn edit_distance(a: &[&str], b: &[&str]) -> Option<usize> {
    let (n, m) = (a.len(), b.len());
    let most = (n + m).min(MOST_CHANGED);
    if n.abs_diff(m) > most {
        return None;
    }
    let offset = most + 1;
    let mut furthest = vec![0usize; 2 * most + 3];
    for d in 0..=most {
        let d = d as isize;
        let mut k = -d;
        while k <= d {
            let at = (k + offset as isize) as usize;
            let mut x = if k == -d || (k != d && furthest[at - 1] < furthest[at + 1]) {
                furthest[at + 1]
            } else {
                furthest[at - 1] + 1
            };
            let mut y = (x as isize - k) as usize;
            while x < n && y < m {
                if a[x] != b[y] {
                    break;
                }
                x += 1;
                y += 1;
            }
            furthest[at] = x;
            if x >= n && y >= m {
                return Some(d as usize);
            }
            k += 2;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::changed_lines;

    #[test]
    fn an_edit_counts_the_lines_it_put_in_and_took_out() {
        assert_eq!(changed_lines("a\nb\nc\n", "a\nb\nc\n"), Some((0, 0)));
        assert_eq!(changed_lines("", "one\ntwo\nthree\n"), Some((3, 0)));
        assert_eq!(changed_lines("one\ntwo\n", ""), Some((0, 2)));
        // One line rewritten is one out and one in.
        assert_eq!(changed_lines("a\nb\nc\n", "a\nB\nc\n"), Some((1, 1)));
        // Two in and one out, in the middle of what stayed.
        assert_eq!(
            changed_lines(
                "fn a() {\n    old();\n}\n",
                "fn a() {\n    new();\n    more();\n}\n"
            ),
            Some((2, 1))
        );
        // A line moved is one out and one in.
        assert_eq!(changed_lines("x\ny\nz\n", "y\nz\nx\n"), Some((1, 1)));
    }
}
