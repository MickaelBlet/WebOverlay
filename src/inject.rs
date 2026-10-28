//! Builds the initialization scripts injected into every page of an app.
//!
//! Each script/style becomes its own init script so that a syntax error in one
//! user file does not prevent the others from running.

use std::fs;

use anyhow::{Context, Result};

use crate::config::{AppConfig, RunAt};

/// Navigation shortcuts handled in the page itself (the content webview has no IPC).
const SHORTCUTS_SCRIPT: &str = r#"window.addEventListener("keydown", (e) => {
  let action;
  if (e.altKey && !e.ctrlKey && e.key === "ArrowLeft") action = () => history.back();
  else if (e.altKey && !e.ctrlKey && e.key === "ArrowRight") action = () => history.forward();
  else if (e.key === "F5" || (e.ctrlKey && !e.altKey && e.key.toLowerCase() === "r")) action = () => location.reload();
  if (!action) return;
  e.preventDefault();
  e.stopImmediatePropagation();
  action();
}, true);
"#;

pub fn build_scripts(app: &AppConfig) -> Result<Vec<String>> {
    let mut out = vec![SHORTCUTS_SCRIPT.to_owned()];
    for (i, style) in app.styles.iter().enumerate() {
        let css = fs::read_to_string(&style.path)
            .with_context(|| format!("cannot read style {}", style.path.display()))?;
        out.push(style_script(
            &css,
            &style.matches,
            &format!("{}-style-{i}", app.name),
        ));
    }
    for script in &app.scripts {
        let js = fs::read_to_string(&script.path)
            .with_context(|| format!("cannot read script {}", script.path.display()))?;
        let label = script.path.display().to_string();
        out.push(user_script(&js, &script.matches, script.run_at, &label));
    }
    Ok(out)
}

/// Converts `*` wildcards into an anchored JS regex source.
fn wildcard_to_regex(pattern: &str) -> String {
    let mut re = String::from("^");
    for c in pattern.chars() {
        match c {
            '*' => re.push_str(".*"),
            '\\' | '^' | '$' | '.' | '|' | '?' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '/' => {
                re.push('\\');
                re.push(c);
            }
            _ => re.push(c),
        }
    }
    re.push('$');
    re
}

fn match_guard(matches: &[String]) -> String {
    if matches.is_empty() {
        return String::new();
    }
    let patterns: Vec<String> = matches.iter().map(|m| wildcard_to_regex(m)).collect();
    let json = serde_json::to_string(&patterns).expect("strings serialize");
    format!("if (!{json}.some((p) => new RegExp(p, \"i\").test(location.href))) return;\n")
}

fn js_string(s: &str) -> String {
    serde_json::to_string(s).expect("strings serialize")
}

fn style_script(css: &str, matches: &[String], id: &str) -> String {
    format!(
        r#"(() => {{
{guard}const css = {css};
try {{
  // Constructable stylesheets are not subject to the page's style-src CSP.
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(css);
  document.adoptedStyleSheets = [...document.adoptedStyleSheets, sheet];
  return;
}} catch (_) {{}}
const insert = () => {{
  const root = document.head || document.documentElement;
  if (!root) return false;
  const el = document.createElement("style");
  el.id = {id};
  el.textContent = css;
  root.appendChild(el);
  return true;
}};
if (!insert()) {{
  const observer = new MutationObserver(() => {{ if (insert()) observer.disconnect(); }});
  observer.observe(document, {{ childList: true, subtree: true }});
}}
}})();
"#,
        guard = match_guard(matches),
        css = js_string(css),
        id = js_string(id),
    )
}

fn user_script(js: &str, matches: &[String], run_at: RunAt, label: &str) -> String {
    let schedule = match run_at {
        RunAt::DocumentStart => "run();",
        RunAt::DocumentEnd => {
            r#"if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", run, { once: true }); else run();"#
        }
        RunAt::DocumentIdle => {
            r#"if (document.readyState === "complete") run(); else window.addEventListener("load", run, { once: true });"#
        }
    };
    format!(
        r#"(() => {{
{guard}const run = () => {{
  try {{
{js}
  }} catch (e) {{
    console.error("[weboverlay] " + {label}, e);
  }}
}};
{schedule}
}})();
"#,
        guard = match_guard(matches),
        label = js_string(label),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regex_escapes_specials() {
        assert_eq!(
            wildcard_to_regex("https://a.com/*?x=(1)"),
            r"^https:\/\/a\.com\/.*\?x=\(1\)$"
        );
    }

    #[test]
    fn css_is_escaped_as_js_string() {
        let css = "a::after { content: \"`${x}`\\\" </script>\"; }\n";
        let script = style_script(css, &[], "s");
        let literal = js_string(css);
        assert!(script.contains(&literal));
        // The CSS must round-trip through the JSON/JS string literal unchanged.
        let back: String = serde_json::from_str(&literal).unwrap();
        assert_eq!(back, css);
    }

    #[test]
    fn guard_only_when_matches() {
        assert!(!user_script("1", &[], RunAt::DocumentEnd, "x").contains("location.href"));
        let s = user_script("1", &["https://a/*".into()], RunAt::DocumentIdle, "x");
        assert!(s.contains("location.href"));
        assert!(s.contains("\"load\""));
    }

    #[test]
    fn trailing_line_comment_does_not_break_wrapper() {
        let s = user_script("console.log(1) // end", &[], RunAt::DocumentStart, "x");
        assert!(s.contains("console.log(1) // end\n  }"));
    }
}
