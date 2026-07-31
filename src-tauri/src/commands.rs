use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::process::Stdio;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

use crate::process;
use crate::types::*;
use crate::util::detect_framework;
use tauri_plugin_autostart::ManagerExt;

#[cfg(windows)]
use crate::process::CREATE_NO_WINDOW_FLAG;
#[cfg(windows)]
use std::os::windows::process::CommandExt;

// ── Dialog ─────────────────────────────────────────

#[tauri::command]
pub fn pick_folder(app: AppHandle) -> Result<Option<String>, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog()
        .file()
        .set_title("Select project folder")
        .pick_folder(move |folder| {
            let path_str =
                folder.and_then(|f| f.as_path().map(|p| p.to_string_lossy().to_string()));
            let _ = tx.send(path_str);
        });
    rx.recv().map_err(|e| e.to_string())
}

// ── Package scanning ───────────────────────────────

const SKIP_SCRIPTS: &[&str] = &["prepare", "prepublishOnly", "postinstall", "preinstall"];

#[tauri::command]
pub fn scan_project(directory: String) -> Result<ScanResult, String> {
    let dir = PathBuf::from(&directory);

    // Try each project type in order.
    // WordPress first: a WP root can also contain package.json (theme tooling).
    if dir.join("wp-config.php").exists()
        || (dir.join("wp-content").is_dir() && dir.join("wp-includes").is_dir())
    {
        return scan_wordpress(&dir);
    }
    if dir.join("deno.json").exists() || dir.join("deno.jsonc").exists() {
        return scan_deno(&dir);
    }
    if dir.join("package.json").exists() {
        return scan_node(&dir);
    }
    if dir.join("Cargo.toml").exists() {
        return scan_cargo(&dir);
    }
    if dir.join("pyproject.toml").exists()
        || dir.join("requirements.txt").exists()
        || dir.join("setup.py").exists()
        || dir.join("Pipfile").exists()
    {
        return scan_python(&dir);
    }
    if dir.join("composer.json").exists() {
        return scan_php(&dir);
    }
    if dir.join("go.mod").exists() {
        return scan_go(&dir);
    }
    if dir.join("Gemfile").exists() {
        return scan_ruby(&dir);
    }
    if dir.join("mix.exs").exists() {
        return scan_elixir(&dir);
    }
    if dir.join("pubspec.yaml").exists() {
        return scan_dart(&dir);
    }
    if dir.join("pom.xml").exists() {
        return scan_maven(&dir);
    }
    if dir.join("build.gradle").exists() || dir.join("build.gradle.kts").exists() {
        return scan_gradle(&dir);
    }
    if let Some(result) = scan_dotnet(&dir) {
        return Ok(result);
    }
    if dir.join("CMakeLists.txt").exists() {
        return scan_cmake(&dir);
    }
    if dir.join("build.zig").exists() {
        return scan_zig(&dir);
    }
    if dir.join("Makefile").exists() || dir.join("makefile").exists() {
        return scan_makefile(&dir);
    }
    if dir.join("docker-compose.yml").exists() || dir.join("docker-compose.yaml").exists() {
        return scan_docker_compose(&dir);
    }

    // Fallback: use folder name, no commands
    let name = dir_name(&dir);
    Ok(ScanResult { name, framework: None, commands: vec![] })
}

fn dir_name(dir: &Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

// ── Node.js / Bun ──────────────────────────────────

fn scan_node(dir: &Path) -> Result<ScanResult, String> {
    let data = fs::read_to_string(dir.join("package.json")).map_err(|e| e.to_string())?;
    let pkg: serde_json::Value = serde_json::from_str(&data).map_err(|e| e.to_string())?;

    let name = pkg.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
    // No known framework → still show the language so the badge isn't empty
    let framework = detect_framework(&pkg).or_else(|| {
        if dir.join("tsconfig.json").exists() {
            Some("TypeScript".into())
        } else {
            Some("JavaScript".into())
        }
    });

    // Detect package manager
    let runner = if dir.join("bun.lockb").exists() || dir.join("bun.lock").exists() {
        "bun run"
    } else if dir.join("pnpm-lock.yaml").exists() {
        "pnpm run"
    } else if dir.join("yarn.lock").exists() {
        "yarn"
    } else {
        "npm run"
    };

    let mut commands = Vec::new();
    if let Some(scripts) = pkg.get("scripts").and_then(|v| v.as_object()) {
        for (key, val) in scripts {
            if SKIP_SCRIPTS.contains(&key.as_str()) || is_lifecycle_hook(key) {
                continue;
            }
            if val.is_string() {
                commands.push(CommandDef {
                    label: key.clone(),
                    cmd: format!("{} {}", runner, key),
                });
            }
        }
    }

    commands.sort_by_key(|c| match c.label.as_str() {
        "dev" => 0, "start" => 1, "serve" => 2,
        "build" => 3, "test" => 4, "lint" => 5,
        _ => 10,
    });

    Ok(ScanResult { name, framework, commands })
}

// ── Rust / Cargo ───────────────────────────────────

fn scan_cargo(dir: &Path) -> Result<ScanResult, String> {
    let data = fs::read_to_string(dir.join("Cargo.toml")).map_err(|e| e.to_string())?;
    let name = data.lines()
        .find(|l| l.starts_with("name"))
        .and_then(|l| l.split('"').nth(1))
        .unwrap_or("")
        .to_string();

    Ok(ScanResult {
        name,
        framework: Some("Rust".into()),
        commands: vec![
            CommandDef { label: "run".into(), cmd: "cargo run".into() },
            CommandDef { label: "build".into(), cmd: "cargo build".into() },
            CommandDef { label: "test".into(), cmd: "cargo test".into() },
            CommandDef { label: "check".into(), cmd: "cargo check".into() },
        ],
    })
}

// ── Python ─────────────────────────────────────────

fn scan_python(dir: &Path) -> Result<ScanResult, String> {
    let name = dir.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let mut framework = None;
    let mut commands = Vec::new();

    // Gather dependency hints from pyproject.toml / requirements.txt / Pipfile
    let mut deps = String::new();
    for f in &["pyproject.toml", "requirements.txt", "Pipfile"] {
        if let Ok(data) = fs::read_to_string(dir.join(f)) {
            deps.push_str(&data.to_lowercase());
        }
    }

    if deps.contains("django") || dir.join("manage.py").exists() {
        framework = Some("Django".into());
        commands.push(CommandDef { label: "dev".into(), cmd: "python manage.py runserver".into() });
        commands.push(CommandDef { label: "migrate".into(), cmd: "python manage.py migrate".into() });
    } else if deps.contains("fastapi") || deps.contains("uvicorn") {
        framework = Some("FastAPI".into());
        commands.push(CommandDef { label: "dev".into(), cmd: "uvicorn main:app --reload".into() });
    } else if deps.contains("flask") {
        framework = Some("Flask".into());
        commands.push(CommandDef { label: "dev".into(), cmd: "flask run --reload".into() });
    } else if deps.contains("streamlit") {
        framework = Some("Streamlit".into());
        commands.push(CommandDef { label: "dev".into(), cmd: "streamlit run app.py".into() });
    }

    if commands.is_empty() {
        commands.push(CommandDef { label: "run".into(), cmd: "python main.py".into() });
    }

    Ok(ScanResult { name, framework: framework.or_else(|| Some("Python".into())), commands })
}

// ── Go ─────────────────────────────────────────────

fn scan_go(dir: &Path) -> Result<ScanResult, String> {
    let data = fs::read_to_string(dir.join("go.mod")).map_err(|e| e.to_string())?;
    let name = data.lines()
        .find(|l| l.starts_with("module"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|m| m.rsplit('/').next())
        .unwrap_or("")
        .to_string();

    Ok(ScanResult {
        name,
        framework: Some("Go".into()),
        commands: vec![
            CommandDef { label: "run".into(), cmd: "go run .".into() },
            CommandDef { label: "build".into(), cmd: "go build .".into() },
            CommandDef { label: "test".into(), cmd: "go test ./...".into() },
        ],
    })
}

// ── Makefile ───────────────────────────────────────

fn scan_makefile(dir: &Path) -> Result<ScanResult, String> {
    let name = dir.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let path = if dir.join("Makefile").exists() {
        dir.join("Makefile")
    } else {
        dir.join("makefile")
    };

    let mut commands = Vec::new();
    if let Ok(data) = fs::read_to_string(&path) {
        for line in data.lines() {
            // Match "target:" or "target: deps"
            if let Some(colon_pos) = line.find(':') {
                let target = line[..colon_pos].trim();
                if !target.is_empty()
                    && !target.starts_with('.')
                    && !target.starts_with('\t')
                    && !target.starts_with(' ')
                    && !target.contains('=')
                    && !target.contains('$')
                {
                    commands.push(CommandDef {
                        label: target.to_string(),
                        cmd: format!("make {}", target),
                    });
                }
            }
        }
    }

    Ok(ScanResult { name, framework: None, commands })
}

// ── PHP / Composer ─────────────────────────────────

fn scan_php(dir: &Path) -> Result<ScanResult, String> {
    let data = fs::read_to_string(dir.join("composer.json")).map_err(|e| e.to_string())?;
    let pkg: serde_json::Value = serde_json::from_str(&data).map_err(|e| e.to_string())?;

    let name = pkg.get("name")
        .and_then(|v| v.as_str())
        .and_then(|n| n.rsplit('/').next())
        .unwrap_or("")
        .to_string();

    let require = pkg.get("require").and_then(|v| v.as_object());
    let require_dev = pkg.get("require-dev").and_then(|v| v.as_object());
    let has = |dep: &str| -> bool {
        require.is_some_and(|d| d.contains_key(dep))
            || require_dev.is_some_and(|d| d.contains_key(dep))
    };

    let framework = if has("laravel/framework") {
        Some("Laravel".into())
    } else if has("symfony/framework-bundle") || has("symfony/console") {
        Some("Symfony".into())
    } else if has("wordpress/core")
        || has("johnpbloch/wordpress")
        || has("roots/wordpress")
        || dir.join("wp-config.php").exists()
        || dir.join("style.css").exists() && dir.join("functions.php").exists() // WP theme
    {
        Some("WordPress".into())
    } else if has("slim/slim") {
        Some("Slim".into())
    } else if has("cakephp/cakephp") {
        Some("CakePHP".into())
    } else {
        Some("PHP".into())
    };

    let mut commands = Vec::new();

    // Add composer scripts
    if let Some(scripts) = pkg.get("scripts").and_then(|v| v.as_object()) {
        for key in scripts.keys() {
            if !key.starts_with("pre-") && !key.starts_with("post-") {
                commands.push(CommandDef {
                    label: key.clone(),
                    cmd: format!("composer {}", key),
                });
            }
        }
    }

    // Add framework-specific commands
    if has("laravel/framework") {
        if !commands.iter().any(|c| c.label == "dev") {
            commands.insert(0, CommandDef { label: "dev".into(), cmd: "php artisan serve".into() });
        }
        commands.push(CommandDef { label: "migrate".into(), cmd: "php artisan migrate".into() });
        commands.push(CommandDef { label: "tinker".into(), cmd: "php artisan tinker".into() });
    } else if has("symfony/framework-bundle") {
        if !commands.iter().any(|c| c.label == "dev") {
            commands.insert(0, CommandDef { label: "dev".into(), cmd: "symfony serve".into() });
        }
    } else if commands.is_empty() {
        commands.push(CommandDef { label: "serve".into(), cmd: "php -S localhost:8000".into() });
    }

    Ok(ScanResult { name, framework, commands })
}

// ── Docker Compose ─────────────────────────────────

fn scan_docker_compose(dir: &Path) -> Result<ScanResult, String> {
    let name = dir.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    Ok(ScanResult {
        name,
        framework: Some("Docker".into()),
        commands: vec![
            CommandDef { label: "up".into(), cmd: "docker compose up".into() },
            CommandDef { label: "up -d".into(), cmd: "docker compose up -d".into() },
            CommandDef { label: "down".into(), cmd: "docker compose down".into() },
            CommandDef { label: "logs".into(), cmd: "docker compose logs -f".into() },
            CommandDef { label: "build".into(), cmd: "docker compose build".into() },
        ],
    })
}

// ── WordPress ──────────────────────────────────────

fn scan_wordpress(dir: &Path) -> Result<ScanResult, String> {
    // A WP root may also carry package.json (theme/block tooling) — keep
    // those npm scripts but brand the project as WordPress.
    let mut result = if dir.join("package.json").exists() {
        scan_node(dir).unwrap_or(ScanResult {
            name: dir_name(dir),
            framework: None,
            commands: vec![],
        })
    } else {
        ScanResult { name: dir_name(dir), framework: None, commands: vec![] }
    };

    if result.name.is_empty() {
        result.name = dir_name(dir);
    }
    result.framework = Some("WordPress".into());

    let has_serve = result.commands.iter().any(|c| {
        matches!(c.label.as_str(), "dev" | "serve" | "start")
    });
    if !has_serve {
        result.commands.insert(0, CommandDef {
            label: "serve".into(),
            cmd: "php -S localhost:8080".into(),
        });
    }
    Ok(result)
}

// ── Deno ───────────────────────────────────────────

fn scan_deno(dir: &Path) -> Result<ScanResult, String> {
    let mut commands = Vec::new();
    for f in &["deno.json", "deno.jsonc"] {
        if let Ok(data) = fs::read_to_string(dir.join(f)) {
            // jsonc: tolerate parse failure, fall through to defaults
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) {
                if let Some(tasks) = json.get("tasks").and_then(|v| v.as_object()) {
                    for key in tasks.keys() {
                        commands.push(CommandDef {
                            label: key.clone(),
                            cmd: format!("deno task {}", key),
                        });
                    }
                }
            }
            break;
        }
    }
    if commands.is_empty() {
        commands.push(CommandDef { label: "run".into(), cmd: "deno run -A main.ts".into() });
        commands.push(CommandDef { label: "test".into(), cmd: "deno test".into() });
    }
    Ok(ScanResult { name: dir_name(dir), framework: Some("Deno".into()), commands })
}

// ── Ruby ───────────────────────────────────────────

fn scan_ruby(dir: &Path) -> Result<ScanResult, String> {
    let gemfile = fs::read_to_string(dir.join("Gemfile")).unwrap_or_default().to_lowercase();

    let (framework, commands) = if gemfile.contains("rails") || dir.join("bin/rails").exists() {
        ("Rails", vec![
            CommandDef { label: "dev".into(), cmd: "bin/rails server".into() },
            CommandDef { label: "console".into(), cmd: "bin/rails console".into() },
            CommandDef { label: "migrate".into(), cmd: "bin/rails db:migrate".into() },
            CommandDef { label: "test".into(), cmd: "bin/rails test".into() },
        ])
    } else if gemfile.contains("jekyll") {
        ("Jekyll", vec![
            CommandDef { label: "dev".into(), cmd: "bundle exec jekyll serve".into() },
            CommandDef { label: "build".into(), cmd: "bundle exec jekyll build".into() },
        ])
    } else if gemfile.contains("sinatra") {
        ("Sinatra", vec![
            CommandDef { label: "dev".into(), cmd: "bundle exec ruby app.rb".into() },
        ])
    } else {
        ("Ruby", vec![
            CommandDef { label: "install".into(), cmd: "bundle install".into() },
        ])
    };

    Ok(ScanResult { name: dir_name(dir), framework: Some(framework.into()), commands })
}

// ── Elixir ─────────────────────────────────────────

fn scan_elixir(dir: &Path) -> Result<ScanResult, String> {
    let mix = fs::read_to_string(dir.join("mix.exs")).unwrap_or_default();

    let (framework, commands) = if mix.contains(":phoenix") {
        ("Phoenix", vec![
            CommandDef { label: "dev".into(), cmd: "mix phx.server".into() },
            CommandDef { label: "deps".into(), cmd: "mix deps.get".into() },
            CommandDef { label: "test".into(), cmd: "mix test".into() },
        ])
    } else {
        ("Elixir", vec![
            CommandDef { label: "run".into(), cmd: "mix run".into() },
            CommandDef { label: "test".into(), cmd: "mix test".into() },
        ])
    };

    Ok(ScanResult { name: dir_name(dir), framework: Some(framework.into()), commands })
}

// ── Dart / Flutter ─────────────────────────────────

fn scan_dart(dir: &Path) -> Result<ScanResult, String> {
    let pubspec = fs::read_to_string(dir.join("pubspec.yaml")).unwrap_or_default();
    let name = pubspec.lines()
        .find(|l| l.starts_with("name:"))
        .map(|l| l.trim_start_matches("name:").trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| dir_name(dir));

    if pubspec.contains("flutter") {
        Ok(ScanResult {
            name,
            framework: Some("Flutter".into()),
            commands: vec![
                CommandDef { label: "run".into(), cmd: "flutter run".into() },
                CommandDef { label: "test".into(), cmd: "flutter test".into() },
                CommandDef { label: "build".into(), cmd: "flutter build apk".into() },
            ],
        })
    } else {
        Ok(ScanResult {
            name,
            framework: Some("Dart".into()),
            commands: vec![
                CommandDef { label: "run".into(), cmd: "dart run".into() },
                CommandDef { label: "test".into(), cmd: "dart test".into() },
            ],
        })
    }
}

// ── Java / Kotlin (Maven & Gradle) ─────────────────

/// Platform-correct wrapper invocation, falling back to the global tool.
fn build_tool(dir: &Path, wrapper: &str, fallback: &str) -> String {
    #[cfg(windows)]
    let candidate = format!("{}.bat", wrapper);
    #[cfg(not(windows))]
    let candidate = format!("./{}", wrapper);

    let exists = {
        #[cfg(windows)]
        { dir.join(format!("{}.bat", wrapper)).exists() || dir.join(format!("{}.cmd", wrapper)).exists() }
        #[cfg(not(windows))]
        { dir.join(wrapper).exists() }
    };
    if exists { candidate } else { fallback.to_string() }
}

fn scan_maven(dir: &Path) -> Result<ScanResult, String> {
    let pom = fs::read_to_string(dir.join("pom.xml")).unwrap_or_default();
    // mvnw wrapper ships as mvnw.cmd on Windows
    let mvn = {
        #[cfg(windows)]
        { if dir.join("mvnw.cmd").exists() { "mvnw.cmd".to_string() } else { "mvn".to_string() } }
        #[cfg(not(windows))]
        { if dir.join("mvnw").exists() { "./mvnw".to_string() } else { "mvn".to_string() } }
    };

    let (framework, mut commands) = if pom.contains("spring-boot") {
        ("Spring Boot", vec![
            CommandDef { label: "dev".into(), cmd: format!("{} spring-boot:run", mvn) },
        ])
    } else {
        ("Java", vec![])
    };
    commands.push(CommandDef { label: "build".into(), cmd: format!("{} package", mvn) });
    commands.push(CommandDef { label: "test".into(), cmd: format!("{} test", mvn) });

    Ok(ScanResult { name: dir_name(dir), framework: Some(framework.into()), commands })
}

fn scan_gradle(dir: &Path) -> Result<ScanResult, String> {
    let mut build = fs::read_to_string(dir.join("build.gradle")).unwrap_or_default();
    build.push_str(&fs::read_to_string(dir.join("build.gradle.kts")).unwrap_or_default());
    let gradle = build_tool(dir, "gradlew", "gradle");

    let (framework, mut commands) = if build.contains("com.android") {
        ("Android", vec![
            CommandDef { label: "build".into(), cmd: format!("{} assembleDebug", gradle) },
            CommandDef { label: "install".into(), cmd: format!("{} installDebug", gradle) },
        ])
    } else if build.contains("spring-boot") || build.contains("org.springframework") {
        ("Spring Boot", vec![
            CommandDef { label: "dev".into(), cmd: format!("{} bootRun", gradle) },
            CommandDef { label: "build".into(), cmd: format!("{} build", gradle) },
        ])
    } else {
        let lang = if dir.join("build.gradle.kts").exists() || build.contains("kotlin") {
            "Kotlin"
        } else {
            "Java"
        };
        (lang, vec![
            CommandDef { label: "run".into(), cmd: format!("{} run", gradle) },
            CommandDef { label: "build".into(), cmd: format!("{} build", gradle) },
        ])
    };
    commands.push(CommandDef { label: "test".into(), cmd: format!("{} test", gradle) });

    Ok(ScanResult { name: dir_name(dir), framework: Some(framework.into()), commands })
}

// ── .NET ───────────────────────────────────────────

/// Returns Some(..) only when the folder actually contains a .sln/.csproj/.fsproj.
fn scan_dotnet(dir: &Path) -> Option<ScanResult> {
    let mut csproj_content = String::new();
    let mut found = false;

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            match path.extension().and_then(|e| e.to_str()) {
                Some("sln") | Some("fsproj") => found = true,
                Some("csproj") => {
                    found = true;
                    if let Ok(data) = fs::read_to_string(&path) {
                        csproj_content.push_str(&data);
                    }
                }
                _ => {}
            }
        }
    }
    if !found {
        return None;
    }

    let framework = if csproj_content.contains("Microsoft.NET.Sdk.Web") {
        "ASP.NET Core"
    } else if csproj_content.contains("Microsoft.NET.Sdk.BlazorWebAssembly") {
        "Blazor"
    } else {
        ".NET"
    };

    Some(ScanResult {
        name: dir_name(dir),
        framework: Some(framework.into()),
        commands: vec![
            CommandDef { label: "run".into(), cmd: "dotnet run".into() },
            CommandDef { label: "watch".into(), cmd: "dotnet watch".into() },
            CommandDef { label: "build".into(), cmd: "dotnet build".into() },
            CommandDef { label: "test".into(), cmd: "dotnet test".into() },
        ],
    })
}

// ── C / C++ (CMake) ────────────────────────────────

fn scan_cmake(dir: &Path) -> Result<ScanResult, String> {
    Ok(ScanResult {
        name: dir_name(dir),
        framework: Some("C/C++".into()),
        commands: vec![
            CommandDef { label: "configure".into(), cmd: "cmake -B build".into() },
            CommandDef { label: "build".into(), cmd: "cmake --build build".into() },
            CommandDef { label: "test".into(), cmd: "ctest --test-dir build".into() },
        ],
    })
}

// ── Zig ────────────────────────────────────────────

fn scan_zig(dir: &Path) -> Result<ScanResult, String> {
    Ok(ScanResult {
        name: dir_name(dir),
        framework: Some("Zig".into()),
        commands: vec![
            CommandDef { label: "run".into(), cmd: "zig build run".into() },
            CommandDef { label: "build".into(), cmd: "zig build".into() },
            CommandDef { label: "test".into(), cmd: "zig build test".into() },
        ],
    })
}

// ── Helpers ────────────────────────────────────────

fn is_lifecycle_hook(key: &str) -> bool {
    for prefix in &["pre", "post"] {
        if let Some(rest) = key.strip_prefix(prefix) {
            if rest.starts_with(|c: char| c.is_uppercase()) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a unique temp project dir with the given files.
    fn fixture(tag: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("onerun-scan-test-{}-{}", std::process::id(), tag));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for (name, content) in files {
            let path = dir.join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, content).unwrap();
        }
        dir
    }

    #[test]
    fn wordpress_root_detected() {
        let dir = fixture("wp", &[("wp-config.php", "<?php")]);
        let r = scan_project(dir.to_string_lossy().to_string()).unwrap();
        assert_eq!(r.framework.as_deref(), Some("WordPress"));
        assert!(r.commands.iter().any(|c| c.cmd.contains("php -S")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn wordpress_wins_over_package_json() {
        let dir = fixture("wp-npm", &[
            ("wp-config.php", "<?php"),
            ("package.json", r#"{"name":"theme","scripts":{"build":"webpack"}}"#),
        ]);
        let r = scan_project(dir.to_string_lossy().to_string()).unwrap();
        assert_eq!(r.framework.as_deref(), Some("WordPress"));
        // npm scripts survive alongside the serve command
        assert!(r.commands.iter().any(|c| c.label == "build"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn plain_node_falls_back_to_javascript() {
        let dir = fixture("js", &[("package.json", r#"{"name":"x","dependencies":{"lodash":"^4"}}"#)]);
        let r = scan_project(dir.to_string_lossy().to_string()).unwrap();
        assert_eq!(r.framework.as_deref(), Some("JavaScript"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn node_with_tsconfig_falls_back_to_typescript() {
        let dir = fixture("ts", &[
            ("package.json", r#"{"name":"x"}"#),
            ("tsconfig.json", "{}"),
        ]);
        let r = scan_project(dir.to_string_lossy().to_string()).unwrap();
        assert_eq!(r.framework.as_deref(), Some("TypeScript"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn plain_python_gets_python_badge() {
        let dir = fixture("py", &[("requirements.txt", "requests\n")]);
        let r = scan_project(dir.to_string_lossy().to_string()).unwrap();
        assert_eq!(r.framework.as_deref(), Some("Python"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rails_detected_from_gemfile() {
        let dir = fixture("rails", &[("Gemfile", "gem 'rails', '~> 7.1'\n")]);
        let r = scan_project(dir.to_string_lossy().to_string()).unwrap();
        assert_eq!(r.framework.as_deref(), Some("Rails"));
        assert!(r.commands.iter().any(|c| c.cmd.contains("rails server")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dotnet_web_detected() {
        let dir = fixture("dotnet", &[(
            "app.csproj",
            r#"<Project Sdk="Microsoft.NET.Sdk.Web"></Project>"#,
        )]);
        let r = scan_project(dir.to_string_lossy().to_string()).unwrap();
        assert_eq!(r.framework.as_deref(), Some("ASP.NET Core"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn deno_tasks_collected() {
        let dir = fixture("deno", &[(
            "deno.json",
            r#"{"tasks":{"dev":"deno run -A --watch main.ts"}}"#,
        )]);
        let r = scan_project(dir.to_string_lossy().to_string()).unwrap();
        assert_eq!(r.framework.as_deref(), Some("Deno"));
        assert!(r.commands.iter().any(|c| c.cmd == "deno task dev"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn flutter_detected_from_pubspec() {
        let dir = fixture("flutter", &[(
            "pubspec.yaml",
            "name: my_app\ndependencies:\n  flutter:\n    sdk: flutter\n",
        )]);
        let r = scan_project(dir.to_string_lossy().to_string()).unwrap();
        assert_eq!(r.framework.as_deref(), Some("Flutter"));
        assert_eq!(r.name, "my_app");
        let _ = fs::remove_dir_all(&dir);
    }
}

// ── Process commands ───────────────────────────────

#[tauri::command]
pub fn start_process(
    id: String,
    command: String,
    label: String,
    cwd: String,
    env: Vec<EnvVar>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    process::start(
        id,
        label,
        command,
        cwd,
        env,
        app,
        state.processes.clone(),
        state.log_viewer.clone(),
    )
}

/// Tell the backend which command's logs are on screen. Reader threads only
/// emit live `process-log` events for this one; pass `None` when the log
/// panel closes.
#[tauri::command]
pub fn set_log_viewer(id: Option<String>, label: Option<String>, state: State<'_, AppState>) {
    let key = match (id, label) {
        (Some(id), Some(label)) => Some(process_key(&id, &label)),
        _ => None,
    };
    if let Ok(mut viewer) = state.log_viewer.lock() {
        *viewer = key;
    }
}

#[tauri::command]
pub fn stop_process(id: String, label: String, app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    process::stop(&id, &label, &state.processes, &app)
}

#[tauri::command]
pub fn stop_all_processes(id: String, app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    process::stop_all(&id, &state.processes, &app)
}

#[tauri::command]
pub fn purge_project(id: String, state: State<'_, AppState>) {
    process::purge_project(&id, &state.processes);
}

// ── Status queries ─────────────────────────────────

#[tauri::command]
pub fn get_logs(id: String, label: String, state: State<'_, AppState>) -> Vec<LogLine> {
    let key = process_key(&id, &label);
    state
        .processes
        .lock()
        .ok()
        .and_then(|map| map.get(&key).map(|ps| ps.logs.iter().cloned().collect()))
        .unwrap_or_default()
}

#[tauri::command]
pub fn get_status(id: String, state: State<'_, AppState>) -> HashMap<String, CmdStatusPayload> {
    let prefix = format!("{}::", id);
    state
        .processes
        .lock()
        .ok()
        .map(|map| {
            map.iter()
                .filter(|(k, _)| k.starts_with(&prefix))
                .map(|(k, ps)| {
                    let (_, label) = parse_key(k);
                    (
                        label.to_string(),
                        CmdStatusPayload {
                            running: ps.running,
                            url: ps.detected_url.clone(),
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

#[tauri::command]
pub fn get_all_status(state: State<'_, AppState>) -> HashMap<String, HashMap<String, CmdStatusPayload>> {
    state
        .processes
        .lock()
        .ok()
        .map(|map| {
            let mut result: HashMap<String, HashMap<String, CmdStatusPayload>> = HashMap::new();
            for (key, ps) in map.iter() {
                let (id, label) = parse_key(key);
                result.entry(id.to_string())
                    .or_default()
                    .insert(label.to_string(), CmdStatusPayload {
                        running: ps.running,
                        url: ps.detected_url.clone(),
                    });
            }
            result
        })
        .unwrap_or_default()
}

// ── OS actions ─────────────────────────────────────

#[tauri::command]
pub fn open_in_explorer(directory: String) -> Result<(), String> {
    #[cfg(windows)]
    {
        Command::new("explorer")
            .arg(&directory)
            .creation_flags(CREATE_NO_WINDOW_FLAG)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(&directory)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(&directory)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn open_in_editor(directory: String, editor: String) -> Result<(), String> {
    #[cfg(windows)]
    {
        Command::new("cmd")
            .args(["/C", &editor, &directory])
            .creation_flags(CREATE_NO_WINDOW_FLAG)
            .stdout(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(windows))]
    {
        Command::new(&editor)
            .arg(&directory)
            .stdout(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn open_in_terminal(directory: String) -> Result<(), String> {
    #[cfg(windows)]
    {
        let escaped = directory.replace('\'', "''");
        Command::new("cmd")
            .args(["/C", "start", "powershell", "-NoExit", "-Command", &format!("Set-Location '{}'", escaped)])
            .creation_flags(CREATE_NO_WINDOW_FLAG)
            .stdout(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .args(["-a", "Terminal", &directory])
            .stdout(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        // Try common terminal emulators in order of preference
        let terminals = ["x-terminal-emulator", "gnome-terminal", "konsole", "xfce4-terminal", "xterm"];
        let mut launched = false;
        for term in &terminals {
            let result = if *term == "gnome-terminal" || *term == "xfce4-terminal" {
                Command::new(term)
                    .args(["--working-directory", &directory])
                    .stdout(Stdio::null())
                    .spawn()
            } else {
                Command::new(term)
                    .current_dir(&directory)
                    .stdout(Stdio::null())
                    .spawn()
            };
            if result.is_ok() {
                launched = true;
                break;
            }
        }
        if !launched {
            return Err("No terminal emulator found".to_string());
        }
    }
    Ok(())
}

#[tauri::command]
pub fn open_in_claude(
    directory: String,
    claude_command: String,
    mode: String,
    project_name: String,
) -> Result<(), String> {
    let tab_title = format!("CLAUDE - {}", project_name);

    #[cfg(windows)]
    {
        if mode == "tab" {
            // Open as new tab in existing Windows Terminal
            // --suppressApplicationTitle prevents Claude from overriding the tab title
            Command::new("cmd")
                .args([
                    "/C", "wt", "-w", "0", "new-tab",
                    "--title", &tab_title,
                    "--suppressApplicationTitle",
                    "-d", &directory,
                    "cmd", "/K", &claude_command,
                ])
                .creation_flags(CREATE_NO_WINDOW_FLAG)
                .spawn()
                .map_err(|e| e.to_string())?;
        } else {
            // Open as new window
            let temp = std::env::temp_dir().join("onerun_claude.bat");
            fs::write(&temp, format!(
                "@echo off\ntitle {}\ncd /d \"{}\"\n{}\n",
                tab_title, directory, claude_command
            )).map_err(|e| e.to_string())?;
            Command::new("cmd")
                .args(["/C", "start", "cmd", "/K", &temp.to_string_lossy().to_string()])
                .creation_flags(CREATE_NO_WINDOW_FLAG)
                .spawn()
                .map_err(|e| e.to_string())?;
        }
    }
    #[cfg(target_os = "macos")]
    {
        let script = if mode == "tab" {
            format!(
                "tell application \"Terminal\"\nactivate\ntell application \"System Events\" to keystroke \"t\" using command down\ndo script \"cd '{}' && {}\" in front window\nend tell",
                directory, claude_command
            )
        } else {
            format!(
                "tell application \"Terminal\" to do script \"cd '{}' && {}\"",
                directory, claude_command
            )
        };
        Command::new("osascript")
            .args(["-e", &script])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        let cmd_str = format!("cd '{}' && {} ; exec bash", directory, claude_command);
        let terminals = [
            ("x-terminal-emulator", vec!["-e", "bash", "-c"]),
            ("gnome-terminal", vec!["--", "bash", "-c"]),
            ("xterm", vec!["-e", "bash", "-c"]),
        ];
        for (term, args) in &terminals {
            let mut c = Command::new(term);
            for a in args { c.arg(a); }
            if c.arg(&cmd_str).spawn().is_ok() {
                return Ok(());
            }
        }
        return Err("No terminal emulator found".into());
    }
    Ok(())
}

#[tauri::command]
pub fn open_in_browser(url: String) -> Result<(), String> {
    #[cfg(windows)]
    {
        Command::new("cmd")
            .args(["/C", "start", "", &url])
            .creation_flags(CREATE_NO_WINDOW_FLAG)
            .stdout(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(&url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ── Window control ─────────────────────────────────

#[tauri::command]
pub fn force_close(app: AppHandle, state: State<'_, AppState>) {
    *state.force_close.lock().unwrap() = true;
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.close();
    }
}

// ── Path validation ───────────────────────────────

#[tauri::command]
pub fn check_paths_exist(paths: Vec<String>) -> HashMap<String, bool> {
    paths.into_iter().map(|p| {
        let exists = Path::new(&p).is_dir();
        (p, exists)
    }).collect()
}

// ── New project folder ─────────────────────────────

fn default_projects_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .document_dir()
        .map(|d| d.join("projects"))
        .unwrap_or_else(|_| PathBuf::from("projects"))
}

#[tauri::command]
pub fn get_default_projects_dir(app: AppHandle) -> String {
    default_projects_dir(&app).to_string_lossy().to_string()
}

#[tauri::command]
pub fn create_project_folder(
    name: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Project name is empty".into());
    }
    if name.contains(['<', '>', ':', '"', '/', '\\', '|', '?', '*']) || name.ends_with('.') {
        return Err("Project name contains invalid characters".into());
    }

    let settings = load_settings(state).unwrap_or_else(|_| Settings::default());
    let parent = if settings.projects_dir.trim().is_empty() {
        default_projects_dir(&app)
    } else {
        PathBuf::from(settings.projects_dir.trim())
    };

    let target = parent.join(name);
    if target.exists() {
        return Err(format!("\"{}\" already exists in {}", name, parent.display()));
    }
    fs::create_dir_all(&target).map_err(|e| e.to_string())?;
    Ok(target.to_string_lossy().to_string())
}

// ── Config persistence ─────────────────────────────

#[tauri::command]
pub fn load_config(state: State<'_, AppState>) -> Result<Vec<ProjectConfig>, String> {
    let path = state.config_path.lock().map_err(|e| e.to_string())?;
    if path.exists() {
        let data = fs::read_to_string(&*path).map_err(|e| e.to_string())?;
        serde_json::from_str(&data).map_err(|e| e.to_string())
    } else {
        Ok(vec![])
    }
}

#[tauri::command]
pub fn save_config(projects: Vec<ProjectConfig>, state: State<'_, AppState>) -> Result<(), String> {
    let path = state.config_path.lock().map_err(|e| e.to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(&projects).map_err(|e| e.to_string())?;
    fs::write(&*path, json).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn load_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    let path = state.settings_path.lock().map_err(|e| e.to_string())?;
    if path.exists() {
        let data = fs::read_to_string(&*path).map_err(|e| e.to_string())?;
        serde_json::from_str(&data).map_err(|e| e.to_string())
    } else {
        Ok(Settings::default())
    }
}

#[tauri::command]
pub fn save_settings(settings: Settings, state: State<'_, AppState>) -> Result<(), String> {
    let path = state.settings_path.lock().map_err(|e| e.to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    fs::write(&*path, json).map_err(|e| e.to_string())
}

// ── Autostart ─────────────────────────────────────

#[tauri::command]
pub fn get_autostart(app: AppHandle) -> Result<bool, String> {
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    let manager = app.autolaunch();
    if enabled {
        manager.enable().map_err(|e| e.to_string())
    } else {
        manager.disable().map_err(|e| e.to_string())
    }
}
