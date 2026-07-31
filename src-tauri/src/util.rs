/// Strip ANSI escape sequences from a string.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_seq = false;
    for c in s.chars() {
        if in_seq {
            if c.is_ascii_alphabetic() {
                in_seq = false;
            }
        } else if c == '\x1b' {
            in_seq = true;
        } else {
            out.push(c);
        }
    }
    out
}

/// How confident we are that a detected URL is the primary browseable one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum UrlConfidence {
    /// Generic "listening on" / bare URL — could be a backend API server.
    Normal = 0,
    /// Explicit frontend label (Vite "Local:", Angular "open your browser", etc.)
    /// — almost certainly the URL you want to open in a browser.
    High = 1,
}

impl Default for UrlConfidence {
    fn default() -> Self {
        Self::Normal
    }
}

/// Try to extract the primary localhost URL from a dev-server log line.
///
/// Three-layer filtering based on how real dev servers print URLs:
///
///  Layer 1 – Line-level skip: reject entire lines that carry a non-primary
///  label (Vite "Debug:", Vite/Next "Network:", CRA "On Your Network:", etc.).
///
///  Layer 2 – Path-level skip: reject URLs whose path begins with `/__`
///  (Vite `/__debug`, Gatsby `/__graphql`, Vite `/__inspect`, etc.).
///
///  Layer 3 – Confidence: lines with a known frontend label ("Local:",
///  "available at", "open your browser") get `High` confidence; everything
///  else gets `Normal`. The caller uses this to avoid letting a backend API
///  URL overwrite a frontend dev-server URL in `concurrently`-style setups.
pub fn detect_url(line: &str) -> Option<(String, UrlConfidence)> {
    // ── Layer 1: skip lines that label a non-primary URL ──────────
    const SKIP_LINE: &[&str] = &[
        "Network:",        // Vite, Next.js, Nuxt, Astro, SvelteKit
        "On Your Network", // CRA / webpack-dev-server
        "Debug:",          // Vite debug panel
        "Inspect:",        // Vite inspect plugin
        "GraphiQL",        // Gatsby GraphiQL explorer
        "graphql",         // Gatsby ___graphql URL line
        "Debugger listening", // Node --inspect
        "Remote:",         // Some HMR/debug tools
    ];

    for skip in SKIP_LINE {
        if line.contains(skip) {
            return None;
        }
    }

    // ── Extract a localhost URL ───────────────────────────────────
    const HOST_PATTERNS: &[&str] = &[
        "http://localhost:",
        "http://127.0.0.1:",
        "http://0.0.0.0:",
        "https://localhost:",
    ];

    for pat in HOST_PATTERNS {
        if let Some(pos) = line.find(pat) {
            let rest = &line[pos..];
            let end = rest
                .find(|c: char| {
                    c.is_whitespace()
                        || c == ','
                        || c == ')'
                        || c == ']'
                        || c == '"'
                        || c == '\''
                        || c == ';'
                        || c == '>'
                })
                .unwrap_or(rest.len());
            let url = rest[..end].trim_end_matches('/');
            if url.len() <= pat.len() {
                continue;
            }

            // ── Layer 2: skip URLs with internal/debug paths ─────
            // Catches /__debug, /___graphql, /__inspect, etc.
            if let Some(slash) = url[pat.len()..].find('/') {
                let path = &url[pat.len() + slash..];
                if path.starts_with("/__") {
                    continue;
                }
            }

            // ── Layer 3: determine confidence ────────────────────
            // Known frontend dev-server labels → High confidence.
            // These appear in: Vite, Next.js, Nuxt, SvelteKit, Astro,
            // CRA ("Local:"), Hugo ("available at"), Angular ("open your browser").
            const HIGH_SIGNALS: &[&str] = &[
                "Local",           // Vite, Next, Nuxt, SvelteKit, Astro, CRA
                "available at",    // Hugo
                "open your browser", // Angular CLI
            ];

            let confidence = if HIGH_SIGNALS.iter().any(|s| line.contains(s)) {
                UrlConfidence::High
            } else {
                UrlConfidence::Normal
            };

            return Some((url.to_string(), confidence));
        }
    }
    None
}

/// Detect the JS framework from package.json dependencies.
pub fn detect_framework(pkg: &serde_json::Value) -> Option<String> {
    let deps = pkg.get("dependencies").and_then(|v| v.as_object());
    let dev_deps = pkg.get("devDependencies").and_then(|v| v.as_object());

    let has = |name: &str| -> bool {
        deps.is_some_and(|d| d.contains_key(name))
            || dev_deps.is_some_and(|d| d.contains_key(name))
    };

    // Order matters — more specific frameworks first
    let frameworks: &[(&dyn Fn() -> bool, &str)] = &[
        // Desktop / mobile shells (checked first — they wrap web frameworks)
        (&|| has("@tauri-apps/api") || has("@tauri-apps/cli"), "Tauri"),
        (&|| has("electron"), "Electron"),
        (&|| has("expo"), "Expo"),
        (&|| has("react-native"), "React Native"),
        (&|| has("@ionic/core") || has("@ionic/react") || has("@ionic/angular") || has("@ionic/vue"), "Ionic"),
        // Meta-frameworks
        (&|| has("next"), "Next.js"),
        (&|| has("nuxt") || has("nuxt3"), "Nuxt"),
        (&|| has("@angular/core"), "Angular"),
        (&|| has("@sveltejs/kit"), "SvelteKit"),
        (&|| has("svelte"), "Svelte"),
        (&|| has("gatsby"), "Gatsby"),
        (&|| has("remix") || has("@remix-run/react") || has("@react-router/dev"), "Remix"),
        (&|| has("astro"), "Astro"),
        (&|| has("solid-start") || has("@solidjs/start"), "SolidStart"),
        (&|| has("solid-js"), "Solid"),
        (&|| has("qwik") || has("@builder.io/qwik"), "Qwik"),
        (&|| has("@redwoodjs/core"), "RedwoodJS"),
        (&|| has("@docusaurus/core"), "Docusaurus"),
        (&|| has("vitepress"), "VitePress"),
        (&|| has("vuepress"), "VuePress"),
        (&|| has("eleventy") || has("@11ty/eleventy"), "11ty"),
        (&|| has("ember-source") || has("ember-cli"), "Ember"),
        // Backend frameworks
        (&|| has("@strapi/strapi") || has("strapi"), "Strapi"),
        (&|| has("@adonisjs/core"), "AdonisJS"),
        (&|| has("hono"), "Hono"),
        (&|| has("elysia"), "Elysia"),
        (&|| has("nest") || has("@nestjs/core"), "NestJS"),
        (&|| has("koa"), "Koa"),
        (&|| has("vite"), "Vite"),
        (&|| has("express"), "Express"),
        (&|| has("fastify"), "Fastify"),
        // UI libraries (last — most specific frameworks include these)
        (&|| has("react-scripts"), "CRA"),
        (&|| has("preact"), "Preact"),
        (&|| has("react"), "React"),
        (&|| has("vue"), "Vue"),
    ];

    frameworks
        .iter()
        .find(|(check, _)| check())
        .map(|(_, name)| (*name).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: extract just the URL string from detect_url.
    fn url(line: &str) -> Option<String> {
        detect_url(line).map(|(u, _)| u)
    }

    /// Helper: extract just the confidence from detect_url.
    fn conf(line: &str) -> Option<UrlConfidence> {
        detect_url(line).map(|(_, c)| c)
    }

    // ── Should detect: HIGH confidence (frontend labels) ─────────

    #[test]
    fn vite_local() {
        let line = "  ➜  Local:   http://localhost:5173/";
        assert_eq!(url(line), Some("http://localhost:5173".into()));
        assert_eq!(conf(line), Some(UrlConfidence::High));
    }

    #[test]
    fn vinext_local() {
        let line = "  ➜  Local:   http://localhost:3000/";
        assert_eq!(url(line), Some("http://localhost:3000".into()));
        assert_eq!(conf(line), Some(UrlConfidence::High));
    }

    #[test]
    fn nextjs_local() {
        let line = "  - Local:        http://localhost:3000";
        assert_eq!(url(line), Some("http://localhost:3000".into()));
        assert_eq!(conf(line), Some(UrlConfidence::High));
    }

    #[test]
    fn cra_local() {
        let line = "  Local:            http://localhost:3000";
        assert_eq!(url(line), Some("http://localhost:3000".into()));
        assert_eq!(conf(line), Some(UrlConfidence::High));
    }

    #[test]
    fn angular_cli() {
        let line = "** Angular Live Development Server is listening on localhost:4200, open your browser on http://localhost:4200/ **";
        assert_eq!(url(line), Some("http://localhost:4200".into()));
        assert_eq!(conf(line), Some(UrlConfidence::High));
    }

    #[test]
    fn hugo() {
        let line = "Web Server is available at http://localhost:1313/";
        assert_eq!(url(line), Some("http://localhost:1313".into()));
        assert_eq!(conf(line), Some(UrlConfidence::High));
    }

    #[test]
    fn astro_local() {
        let line = "  ┃ Local    http://localhost:4321/";
        assert_eq!(url(line), Some("http://localhost:4321".into()));
        assert_eq!(conf(line), Some(UrlConfidence::High));
    }

    #[test]
    fn concurrently_ui_local() {
        let line = "[ui]   ➜  Local:   http://localhost:5173/";
        assert_eq!(url(line), Some("http://localhost:5173".into()));
        assert_eq!(conf(line), Some(UrlConfidence::High));
    }

    // ── Should detect: NORMAL confidence (backend / generic) ─────

    #[test]
    fn gatsby_main() {
        let line = "  http://localhost:8000/";
        assert_eq!(url(line), Some("http://localhost:8000".into()));
        assert_eq!(conf(line), Some(UrlConfidence::Normal));
    }

    #[test]
    fn django() {
        let line = "Starting development server at http://127.0.0.1:8000/";
        assert_eq!(url(line), Some("http://127.0.0.1:8000".into()));
        assert_eq!(conf(line), Some(UrlConfidence::Normal));
    }

    #[test]
    fn flask() {
        let line = " * Running on http://127.0.0.1:5000";
        assert_eq!(url(line), Some("http://127.0.0.1:5000".into()));
        assert_eq!(conf(line), Some(UrlConfidence::Normal));
    }

    #[test]
    fn rails() {
        let line = "* Listening on http://127.0.0.1:3000";
        assert_eq!(url(line), Some("http://127.0.0.1:3000".into()));
        assert_eq!(conf(line), Some(UrlConfidence::Normal));
    }

    #[test]
    fn laravel() {
        let line = "Starting Laravel development server: http://127.0.0.1:8000";
        assert_eq!(url(line), Some("http://127.0.0.1:8000".into()));
        assert_eq!(conf(line), Some(UrlConfidence::Normal));
    }

    #[test]
    fn elysia() {
        let line = "🦊 Elysia is running at http://localhost:3000";
        assert_eq!(url(line), Some("http://localhost:3000".into()));
        assert_eq!(conf(line), Some(UrlConfidence::Normal));
    }

    #[test]
    fn fastify() {
        let line = "Server listening at http://127.0.0.1:3000";
        assert_eq!(url(line), Some("http://127.0.0.1:3000".into()));
        assert_eq!(conf(line), Some(UrlConfidence::Normal));
    }

    #[test]
    fn express_0000() {
        let line = "Listening on http://0.0.0.0:4000";
        assert_eq!(url(line), Some("http://0.0.0.0:4000".into()));
        assert_eq!(conf(line), Some(UrlConfidence::Normal));
    }

    #[test]
    fn concurrently_server() {
        let line = "[server] Server listening on http://localhost:3001";
        assert_eq!(url(line), Some("http://localhost:3001".into()));
        assert_eq!(conf(line), Some(UrlConfidence::Normal));
    }

    // ── Should reject (non-primary URLs) ─────────────────────────

    #[test]
    fn vite_debug() {
        assert_eq!(detect_url("  ➜  Debug:   http://localhost:5173/__debug"), None);
    }

    #[test]
    fn vinext_debug() {
        assert_eq!(detect_url("  ➜  Debug:   http://localhost:3000/__debug"), None);
    }

    #[test]
    fn vite_network() {
        assert_eq!(detect_url("  ➜  Network: http://192.168.1.5:5173/"), None);
    }

    #[test]
    fn nextjs_network() {
        assert_eq!(detect_url("  - Network:      http://192.168.1.5:3000"), None);
    }

    #[test]
    fn cra_network() {
        assert_eq!(detect_url("  On Your Network:  http://192.168.1.5:3000"), None);
    }

    #[test]
    fn gatsby_graphiql() {
        assert_eq!(detect_url("  http://localhost:8000/___graphql"), None);
    }

    #[test]
    fn vite_inspect() {
        assert_eq!(detect_url("  Inspect:  http://localhost:5173/__inspect"), None);
    }

    #[test]
    fn node_debugger() {
        assert_eq!(detect_url("Debugger listening on ws://127.0.0.1:9229/abc-123"), None);
    }

    #[test]
    fn generic_internal_path() {
        assert_eq!(detect_url("  http://localhost:3000/__hmr"), None);
    }

    // ── Confidence ordering: High > Normal ───────────────────────

    #[test]
    fn confidence_ordering() {
        assert!(UrlConfidence::High > UrlConfidence::Normal);
    }

    // ── Concurrently scenario: server + UI ───────────────────────

    #[test]
    fn concurrently_full_scenario() {
        // Simulates the overwrite logic in spawn_reader:
        // first the backend URL arrives (Normal), then the frontend (High).
        // After that, more backend output should NOT replace the frontend URL.
        let lines = [
            "[server] Server listening on http://localhost:3001",
            "[server] WebSocket available at ws://localhost:3001",
            "[ui]   ➜  Local:   http://localhost:5173/",
            "[ui]   ➜  Network: use --host to expose",
        ];

        let mut stored_url: Option<String> = None;
        let mut stored_conf = UrlConfidence::Normal;

        for line in &lines {
            if let Some((u, c)) = detect_url(line) {
                let dominated = stored_url.is_some() && c < stored_conf;
                let unchanged = stored_url.as_ref() == Some(&u);
                if !dominated && !unchanged {
                    stored_url = Some(u);
                    stored_conf = c;
                }
            }
        }

        // The UI's Local URL wins and sticks.
        assert_eq!(stored_url, Some("http://localhost:5173".into()));
        assert_eq!(stored_conf, UrlConfidence::High);
    }

    #[test]
    fn concurrently_reverse_order() {
        // Even if the UI prints first and the server prints second,
        // the High-confidence UI URL must not be overwritten.
        let lines = [
            "[ui]   ➜  Local:   http://localhost:5173/",
            "[server] Server listening on http://localhost:3001",
        ];

        let mut stored_url: Option<String> = None;
        let mut stored_conf = UrlConfidence::Normal;

        for line in &lines {
            if let Some((u, c)) = detect_url(line) {
                let dominated = stored_url.is_some() && c < stored_conf;
                let unchanged = stored_url.as_ref() == Some(&u);
                if !dominated && !unchanged {
                    stored_url = Some(u);
                    stored_conf = c;
                }
            }
        }

        assert_eq!(stored_url, Some("http://localhost:5173".into()));
        assert_eq!(stored_conf, UrlConfidence::High);
    }

    #[test]
    fn solo_backend_still_detected() {
        // When there's only a backend (no frontend), Normal URL should work fine.
        let line = "Server listening on http://localhost:3001";
        assert_eq!(url(line), Some("http://localhost:3001".into()));
    }
}
