//! Profiling (Build > Profile): the program runs, macOS's `sample` tool takes a few seconds of samples of it,
//! and the functions where it spent its time are listed. The text work is here (UI-free and tested); the
//! run is in `main.rs`.

/// A function the samples found on top of a thread's stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hotspot {
    pub name: String,
    /// The library or program the function is in (`prog`, `libsystem_kernel.dylib`).
    pub library: String,
    pub samples: u64,
}

/// The "Sort by top of stack" section of a `sample` report: the functions the samples were in, most
/// samples first. Lines that are not `name  (in library)  count` are skipped.
pub fn hotspots(report: &str) -> Vec<Hotspot> {
    let mut out = Vec::new();
    let mut in_section = false;
    for line in report.lines() {
        if line.starts_with("Sort by top of stack") {
            in_section = true;
            continue;
        }
        if !in_section {
            continue;
        }
        if line.trim().is_empty() {
            break;
        }
        let t = line.trim();
        let Some((rest, count)) = t.rsplit_once(char::is_whitespace) else { continue };
        let Ok(samples) = count.trim().parse::<u64>() else { continue };
        let rest = rest.trim_end();
        let (name, library) = match rest.rsplit_once("  (in ") {
            Some((name, lib)) => (name.trim().to_string(), lib.trim_end_matches(')').to_string()),
            None => (rest.to_string(), String::new()),
        };
        out.push(Hotspot { name, library, samples });
    }
    out.sort_by(|a, b| b.samples.cmp(&a.samples).then_with(|| a.name.cmp(&b.name)));
    out
}

/// The table for the Output tab: the `top` functions with their samples and their share of all samples
/// taken (the samples in the section, which counts each sample once).
pub fn table(hotspots: &[Hotspot], top: usize) -> String {
    let total: u64 = hotspots.iter().map(|h| h.samples).sum();
    let mut out = format!("{:>8}  {:>6}  function (library)\n", "samples", "share");
    for h in hotspots.iter().take(top) {
        let share = if total == 0 { 0.0 } else { h.samples as f64 * 100.0 / total as f64 };
        let place = if h.library.is_empty() { String::new() } else { format!(" ({})", h.library) };
        out.push_str(&format!("{:>8}  {:>5.1}%  {}{}\n", h.samples, share, h.name, place));
    }
    if hotspots.is_empty() {
        out.push_str("no samples were taken in functions the report names\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPORT: &str = "Analysis of sampling prog (pid 42) every 1 millisecond\nCall graph:\n    100 Thread_1\nTotal number in stack (recursive counted multiple, when >=5):\nSort by top of stack, same collapsed (when >= 5):\n        Fibonacci  (in prog)        700\n        __semwait_signal  (in libsystem_kernel.dylib)        120\n        PASCALMAIN  (in prog)        9\n\nBinary Images:\n    0x1 - 0x2 prog\n";

    #[test]
    fn the_top_of_stack_section_gives_the_functions_most_samples_first() {
        let h = hotspots(REPORT);
        assert_eq!(h.len(), 3);
        assert_eq!(h[0], Hotspot { name: "Fibonacci".into(), library: "prog".into(), samples: 700 });
        assert_eq!(h[1].name, "__semwait_signal");
        assert_eq!(h[1].library, "libsystem_kernel.dylib");
        assert!(hotspots("no section here\n").is_empty());
    }

    #[test]
    fn the_table_shows_each_share_of_the_samples() {
        let t = table(&hotspots(REPORT), 2);
        assert!(t.contains("     700   84.4%  Fibonacci (prog)"), "{t}");
        assert!(t.contains("     120   14.5%  __semwait_signal (libsystem_kernel.dylib)"), "{t}");
        assert!(!t.contains("PASCALMAIN"), "only the top two");
    }
}
