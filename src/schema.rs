//! Documentation of the config keys, for the editor's completion and hover help.
//! Keep in sync with `config.rs` (the tests check that every key and value parses).

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct KeyDoc {
    /// `root`, `app`, `app.header`, `script` (`scripts` items) or `style` (`styles` items).
    pub table: &'static str,
    pub key: &'static str,
    #[serde(rename = "type")]
    pub ty: &'static str,
    pub required: bool,
    pub default: Option<&'static str>,
    pub doc: &'static str,
    /// Valid TOML value, also the completion snippet (`$0` = caret).
    pub example: &'static str,
    pub values: &'static [ValueDoc],
}

#[derive(Debug, Serialize)]
pub struct ValueDoc {
    pub value: &'static str,
    pub doc: &'static str,
}

const fn key(
    table: &'static str,
    key: &'static str,
    ty: &'static str,
    default: Option<&'static str>,
    doc: &'static str,
    example: &'static str,
) -> KeyDoc {
    KeyDoc {
        table,
        key,
        ty,
        required: false,
        default,
        doc,
        example,
        values: &[],
    }
}

const RUN_AT: &[ValueDoc] = &[
    ValueDoc {
        value: "document-start",
        doc: "Before the page scripts run (the DOM is still empty).",
    },
    ValueDoc {
        value: "document-end",
        doc: "On `DOMContentLoaded`: the DOM is parsed (default).",
    },
    ValueDoc {
        value: "document-idle",
        doc: "On `load`: images and subresources are loaded too.",
    },
];

const HEADER_ITEMS: &[ValueDoc] = &[
    ValueDoc {
        value: "back",
        doc: "Back button.",
    },
    ValueDoc {
        value: "forward",
        doc: "Forward button.",
    },
    ValueDoc {
        value: "reload",
        doc: "Reload button.",
    },
    ValueDoc {
        value: "home",
        doc: "Home button (goes to `url`).",
    },
    ValueDoc {
        value: "favicon",
        doc: "Site favicon.",
    },
    ValueDoc {
        value: "title",
        doc: "Page title.",
    },
    ValueDoc {
        value: "url",
        doc: "Current URL.",
    },
    ValueDoc {
        value: "zoom",
        doc: "The three zoom buttons (out, reset, in).",
    },
    ValueDoc {
        value: "edit",
        doc: "Edit config button.",
    },
    ValueDoc {
        value: "external",
        doc: "Open in the system browser button.",
    },
    ValueDoc {
        value: "devtools",
        doc: "Developer tools (web inspector / console) button.",
    },
];

const BOOL: &[ValueDoc] = &[
    ValueDoc {
        value: "true",
        doc: "",
    },
    ValueDoc {
        value: "false",
        doc: "",
    },
];

const MATCHES_DOC: &str = "URL wildcards (`*` = any characters) matched against `location.href`, \
    case-insensitive. Empty: every page.";

pub static SCHEMA: &[KeyDoc] = &[
    KeyDoc {
        required: true,
        ..key(
            "root",
            "app",
            "array of tables",
            None,
            "One `[[app]]` table per app window.",
            "[[app]]",
        )
    },
    KeyDoc {
        required: true,
        ..key(
            "app",
            "name",
            "string",
            None,
            "App name, `[A-Za-z0-9_-]+`, unique in the config. Used on the command line \
             (`weboverlay <name>`), in the window label and for the profile directory \
             (cookies, storage).",
            "\"$0\"",
        )
    },
    KeyDoc {
        required: true,
        ..key(
            "app",
            "url",
            "string (http/https URL)",
            None,
            "Start page, also the target of the header's home button.",
            "\"https://$0\"",
        )
    },
    key(
        "app",
        "title",
        "string",
        Some("the page title"),
        "Window title.",
        "\"$0\"",
    ),
    key(
        "app",
        "width",
        "number",
        Some("1200"),
        "Initial window width in logical pixels.",
        "1200$0",
    ),
    key(
        "app",
        "height",
        "number",
        Some("800"),
        "Initial window height in logical pixels.",
        "800$0",
    ),
    KeyDoc {
        values: BOOL,
        ..key(
            "app",
            "maximized",
            "boolean",
            Some("false"),
            "Start maximized (`width`/`height` still apply when restored). Also `--maximized` \
             on the command line.",
            "true$0",
        )
    },
    key(
        "app",
        "zoom",
        "number (0.3 to 3.0)",
        Some("1.0"),
        "Initial page zoom (1.0 = 100%), also the target of the header's zoom reset.",
        "1.25$0",
    ),
    key(
        "app",
        "user_agent",
        "string",
        Some("the WebView user agent"),
        "User agent sent by the content webview.",
        "\"$0\"",
    ),
    key(
        "app",
        "allowed_hosts",
        "array of strings",
        Some("the host of `url`"),
        "Host wildcards kept inside the window; links and popups to other hosts open in the \
         system browser. `*.example.com` also matches `example.com`.",
        "[\"$0\"]",
    ),
    key(
        "app",
        "scripts",
        "array of tables",
        Some("[]"),
        "JavaScript files injected into every page (main frame), each in its own \
         `try/catch`. Items: `path`, `run_at`, `matches`.",
        "[{ path = \"$0\" }]",
    ),
    key(
        "app",
        "styles",
        "array of tables",
        Some("[]"),
        "CSS files injected into every page (main frame). Items: `path`, `matches`.",
        "[{ path = \"$0\" }]",
    ),
    key(
        "app",
        "header",
        "table",
        None,
        "Header bar options, usually written as an `[app.header]` table after the app keys.",
        "{ $0 }",
    ),
    key(
        "app.header",
        "color",
        "string (#rgb or #rrggbb)",
        Some("the system theme"),
        "Header background color.",
        "\"#$0\"",
    ),
    key(
        "app.header",
        "text_color",
        "string (#rgb or #rrggbb)",
        Some("light or dark, following `color`"),
        "Header text and icon color.",
        "\"#$0\"",
    ),
    KeyDoc {
        values: HEADER_ITEMS,
        ..key(
            "app.header",
            "hide",
            "array of strings",
            Some("[]"),
            "Header items not shown (window controls always stay).",
            "[\"$0\"]",
        )
    },
    KeyDoc {
        required: true,
        ..key(
            "script",
            "path",
            "string",
            None,
            "JavaScript file, relative to the config file. Read at startup: restart after \
             editing.",
            "\"$0\"",
        )
    },
    KeyDoc {
        values: RUN_AT,
        ..key(
            "script",
            "run_at",
            "string",
            Some("\"document-end\""),
            "When the script runs.",
            "\"$0\"",
        )
    },
    key(
        "script",
        "matches",
        "array of strings",
        Some("[] (every page)"),
        MATCHES_DOC,
        "[\"$0\"]",
    ),
    KeyDoc {
        required: true,
        ..key(
            "style",
            "path",
            "string",
            None,
            "CSS file, relative to the config file. Read at startup: restart after editing.",
            "\"$0\"",
        )
    },
    key(
        "style",
        "matches",
        "array of strings",
        Some("[] (every page)"),
        MATCHES_DOC,
        "[\"$0\"]",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    /// `key = example` entries of `table` (nested tables skipped), placeholders filled in.
    fn entries(table: &str) -> Vec<String> {
        let fill = |example: &str| {
            example
                .replace("https://$0", "https://a.com")
                .replace("#$0", "#123")
                .replace("$0", "a")
        };
        SCHEMA
            .iter()
            .filter(|k| k.table == table && !matches!(k.key, "scripts" | "styles" | "header"))
            .map(|k| match k.key {
                "run_at" => format!("{} = \"document-idle\"", k.key),
                "hide" => format!("{} = [\"zoom\"]", k.key),
                "width" | "height" | "zoom" | "maximized" => {
                    format!("{} = {}", k.key, k.example.replace("$0", ""))
                }
                _ => format!("{} = {}", k.key, fill(k.example)),
            })
            .collect()
    }

    #[test]
    fn every_key_parses() {
        let text = format!(
            "[[app]]\n{}\nscripts = [{{ {} }}]\nstyles = [{{ {} }}]\n[app.header]\n{}\n",
            entries("app").join("\n"),
            entries("script").join(", "),
            entries("style").join(", "),
            entries("app.header").join("\n"),
        );
        let config = Config::parse(&text).unwrap_or_else(|e| panic!("{e:#}\n{text}"));
        let app = &config.apps[0];
        assert_eq!(app.title.as_deref(), Some("a"));
        assert_eq!(app.scripts[0].matches, ["a"]);
        assert_eq!(app.styles.len(), 1);
        assert!(app.header.text_color.is_some());
        // The snippets of the nested tables are valid TOML values too.
        for key in ["scripts", "styles"] {
            let example = SCHEMA.iter().find(|k| k.key == key).unwrap().example;
            let text = format!(
                "[[app]]\nname = \"a\"\nurl = \"https://a.com\"\n{key} = {}",
                example.replace("$0", "a.js")
            );
            Config::parse(&text).unwrap_or_else(|e| panic!("{key}: {e:#}"));
        }
    }

    #[test]
    fn every_value_parses() {
        let values = |key: &str| {
            SCHEMA
                .iter()
                .find(|k| k.key == key)
                .unwrap()
                .values
                .iter()
                .map(|v| v.value)
                .collect::<Vec<_>>()
        };
        for run_at in values("run_at") {
            let text = format!(
                "[[app]]\nname = \"a\"\nurl = \"https://a.com\"\nscripts = [{{ path = \"a.js\", run_at = \"{run_at}\" }}]"
            );
            Config::parse(&text).unwrap_or_else(|e| panic!("{run_at}: {e:#}"));
        }
        let hide = values("hide");
        let text = format!(
            "[[app]]\nname = \"a\"\nurl = \"https://a.com\"\n[app.header]\nhide = {hide:?}"
        );
        let config = Config::parse(&text).unwrap();
        assert_eq!(config.apps[0].header.hide.len(), hide.len());
        for maximized in values("maximized") {
            let text =
                format!("[[app]]\nname = \"a\"\nurl = \"https://a.com\"\nmaximized = {maximized}");
            Config::parse(&text).unwrap();
        }
    }
}
