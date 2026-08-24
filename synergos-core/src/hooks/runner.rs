//! フック実行本体。daemon 設定のフックと (opt-in なら) プロジェクトフックを
//! 束ねて対象イベント + ファイルにマッチするものを実行する。

use std::path::Path;
use std::time::Duration;

use synergos_net::config::{HookDef, HooksConfig};

use super::project_file;

const MAX_CONCURRENT_POST_HOOK_BATCHES: usize = 16;

/// フックイベント種別。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    PrePublish,
    PostPublish,
    PostReceive,
}

impl HookEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            HookEvent::PrePublish => "pre-publish",
            HookEvent::PostPublish => "post-publish",
            HookEvent::PostReceive => "post-receive",
        }
    }
}

/// フックの定義元。CLI `hooks ls` の表示や、実行元の切り分けに使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookSource {
    /// daemon 設定 (`synergos.toml` の `[hooks]`)。常に有効。
    Daemon,
    /// プロジェクト設定 (`<root>/.synergos/hooks.toml`)。`allow_project_hooks` の opt-in が要る。
    Project,
}

impl HookSource {
    pub fn as_str(self) -> &'static str {
        match self {
            HookSource::Daemon => "daemon",
            HookSource::Project => "project",
        }
    }
}

/// 1 フックの実行結果。
#[derive(Debug, Clone)]
pub struct HookOutcome {
    pub source: HookSource,
    pub command: String,
    pub status: HookStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookStatus {
    /// 正常終了 (exit code 0)。
    Success,
    /// 非 0 exit。
    Failed { exit_code: Option<i32> },
    /// タイムアウトで kill された。
    TimedOut,
    /// spawn に失敗 (コマンドが見つからない等)。
    SpawnError(String),
}

/// `hooks ls` 向け、有効なフック 1 件のサマリ。
#[derive(Debug, Clone)]
pub struct EffectiveHook {
    pub source: HookSource,
    pub def: HookDef,
    /// プロジェクトフックだが `allow_project_hooks = false` で無効化されている。
    pub disabled_by_opt_in: bool,
}

pub struct HookRunner {
    config: HooksConfig,
    post_hook_slots: std::sync::Arc<tokio::sync::Semaphore>,
}

impl HookRunner {
    pub fn new(config: HooksConfig) -> Self {
        Self {
            config,
            post_hook_slots: std::sync::Arc::new(tokio::sync::Semaphore::new(
                MAX_CONCURRENT_POST_HOOK_BATCHES,
            )),
        }
    }

    /// `hooks ls` 用: daemon フック全部 + (opt-in なら) プロジェクトフック全部を列挙する。
    /// opt-in が無効でもプロジェクトフックの存在は見せる (`disabled_by_opt_in = true`)。
    pub async fn effective_hooks(&self, project_root: &Path) -> std::io::Result<Vec<EffectiveHook>> {
        let mut out: Vec<EffectiveHook> = self
            .config
            .hooks
            .iter()
            .cloned()
            .map(|def| EffectiveHook {
                source: HookSource::Daemon,
                def,
                disabled_by_opt_in: false,
            })
            .collect();
        let project_hooks = project_file::load(project_root).await?;
        let allow = self.config.allow_project_hooks;
        out.extend(project_hooks.into_iter().map(|def| EffectiveHook {
            source: HookSource::Project,
            def,
            disabled_by_opt_in: !allow,
        }));
        Ok(out)
    }

    /// `event` + `rel_path` (プロジェクトルート相対, `/` 区切り) にマッチする
    /// 有効なフック (daemon 全部 + opt-in ならプロジェクト) を集める。
    async fn matching_hooks(
        &self,
        project_root: &Path,
        event: HookEvent,
        rel_path: &str,
    ) -> std::io::Result<Vec<(HookSource, HookDef)>> {
        let mut matched: Vec<(HookSource, HookDef)> = self
            .config
            .hooks
            .iter()
            .filter(|def| def.event == event.as_str() && def.matches(rel_path))
            .cloned()
            .map(|def| (HookSource::Daemon, def))
            .collect();
        if self.config.allow_project_hooks {
            let project_hooks = project_file::load(project_root).await?;
            matched.extend(
                project_hooks
                    .into_iter()
                    .filter(|def| def.event == event.as_str() && def.matches(rel_path))
                    .map(|def| (HookSource::Project, def)),
            );
        }
        Ok(matched)
    }

    /// `pre-publish`: 同期待ち。非 0 exit / timeout / spawn 失敗があれば最初の
    /// 失敗を `Err` として返す (publish 中止の理由になる)。バージョンはまだ
    /// 発番前なので `SYNERGOS_VERSION` は設定しない。
    pub async fn run_pre_publish(
        &self,
        project_root: &Path,
        project_id: &str,
        rel_path: &str,
    ) -> std::io::Result<()> {
        let hooks = self
            .matching_hooks(project_root, HookEvent::PrePublish, rel_path)
            .await?;
        for (source, def) in hooks {
            let outcome = execute(
                project_root,
                source,
                &def,
                &Env {
                    event: HookEvent::PrePublish,
                    project_id,
                    rel_path,
                    version: None,
                    peer: None,
                },
            )
            .await;
            match outcome.status {
                HookStatus::Success => {}
                HookStatus::Failed { exit_code } => {
                    return Err(std::io::Error::other(format!(
                        "pre-publish {} hook failed: exit={exit_code:?}",
                        source.as_str()
                    )));
                }
                HookStatus::TimedOut => {
                    return Err(std::io::Error::other(format!(
                        "pre-publish {} hook timed out after {}s",
                        source.as_str(), def.timeout_sec
                    )));
                }
                HookStatus::SpawnError(e) => {
                    return Err(std::io::Error::other(format!(
                        "pre-publish {} hook failed to start: {e}",
                        source.as_str()
                    )));
                }
            }
        }
        Ok(())
    }

    /// `post-publish` / `post-receive`: 転送・イベントループをブロックしないよう
    /// spawn するだけで待たない。失敗は呼び出し側が warn ログに出す。
    pub fn spawn_post_hooks(
        self: &std::sync::Arc<Self>,
        project_root: std::path::PathBuf,
        event: HookEvent,
        project_id: String,
        rel_path: String,
        version: u64,
        peer: Option<String>,
    ) {
        let runner = self.clone();
        tokio::spawn(async move {
            // Bound child-process concurrency while retaining non-blocking behavior for the
            // transfer/event-loop caller. Excess batches wait as lightweight Tokio tasks.
            let Ok(_permit) = runner.post_hook_slots.clone().acquire_owned().await else {
                tracing::warn!("{} hook scheduler closed", event.as_str());
                return;
            };
            let hooks = match runner
                .matching_hooks(&project_root, event, &rel_path)
                .await
            {
                Ok(hooks) => hooks,
                Err(e) => {
                    tracing::warn!(
                        "{} hook lookup failed for {rel_path}: {e}",
                        event.as_str()
                    );
                    return;
                }
            };
            for (source, def) in hooks {
                let outcome = execute(
                    &project_root,
                    source,
                    &def,
                    &Env {
                        event,
                        project_id: &project_id,
                        rel_path: &rel_path,
                        version: Some(version),
                        peer: peer.as_deref(),
                    },
                )
                .await;
                log_outcome(event, source, &outcome);
            }
        });
    }

    /// `synergos-core hooks run` (手動発火 / デバッグ用)。同期待ちで結果を返す。
    pub async fn run_manual(
        &self,
        project_root: &Path,
        event: HookEvent,
        project_id: &str,
        rel_path: &str,
    ) -> std::io::Result<Vec<HookOutcome>> {
        let hooks = self.matching_hooks(project_root, event, rel_path).await?;
        let mut outcomes = Vec::with_capacity(hooks.len());
        for (source, def) in hooks {
            let outcome = execute(
                project_root,
                source,
                &def,
                &Env {
                    event,
                    project_id,
                    rel_path,
                    version: None,
                    peer: None,
                },
            )
            .await;
            outcomes.push(outcome);
        }
        Ok(outcomes)
    }
}

fn log_outcome(event: HookEvent, source: HookSource, outcome: &HookOutcome) {
    match &outcome.status {
        HookStatus::Success => {
            tracing::debug!(
                "{} hook ok ({})",
                event.as_str(),
                source.as_str()
            );
        }
        HookStatus::Failed { exit_code } => {
            tracing::warn!(
                "{} hook failed ({}): exit={:?}",
                event.as_str(),
                source.as_str(),
                exit_code
            );
        }
        HookStatus::TimedOut => {
            tracing::warn!(
                "{} hook timed out ({})",
                event.as_str(),
                source.as_str()
            );
        }
        HookStatus::SpawnError(e) => {
            tracing::warn!(
                "{} hook failed to start ({}): {e}",
                event.as_str(),
                source.as_str()
            );
        }
    }
}

struct Env<'a> {
    event: HookEvent,
    project_id: &'a str,
    rel_path: &'a str,
    version: Option<u64>,
    peer: Option<&'a str>,
}

async fn execute(
    project_root: &Path,
    source: HookSource,
    def: &HookDef,
    env: &Env<'_>,
) -> HookOutcome {
    let mut cmd = shell_command(&def.command);
    cmd.current_dir(project_root);
    cmd.env("SYNERGOS_EVENT", env.event.as_str());
    cmd.env("SYNERGOS_PROJECT", env.project_id);
    cmd.env("SYNERGOS_FILE", env.rel_path);
    if let Some(version) = env.version {
        cmd.env("SYNERGOS_VERSION", version.to_string());
    }
    if let Some(peer) = env.peer {
        cmd.env("SYNERGOS_PEER", peer);
    }
    cmd.stdin(std::process::Stdio::null());
    // Hook output is not consumed. Null sinks prevent an untrusted hook from making the
    // daemon buffer unbounded stdout/stderr via `wait_with_output`.
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());
    cmd.kill_on_drop(true);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Give the hook its own process group so timeout cleanup can terminate descendants.
        cmd.as_std_mut().process_group(0);
    }

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            return HookOutcome {
                source,
                command: def.command.clone(),
                status: HookStatus::SpawnError(e.to_string()),
            };
        }
    };
    let process_id = child.id();

    let timeout = Duration::from_secs(def.timeout_sec.max(1));
    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) => {
            if status.success() {
                HookStatus::Success
            } else {
                HookStatus::Failed {
                    exit_code: status.code(),
                }
            }
        }
        Ok(Err(e)) => HookStatus::SpawnError(e.to_string()),
        Err(_elapsed) => {
            terminate_process_tree(&mut child, process_id).await;
            let _ = child.wait().await;
            HookStatus::TimedOut
        }
    };

    HookOutcome {
        source,
        command: def.command.clone(),
        status,
    }
}

#[cfg(unix)]
async fn terminate_process_tree(child: &mut tokio::process::Child, process_id: Option<u32>) {
    if let Some(process_id) = process_id {
        // SAFETY: the child was placed in a process group whose id equals its pid. A negative
        // pid targets only that group, and SIGKILL requires no Rust-owned pointer or memory.
        let result = unsafe { libc::kill(-(process_id as libc::pid_t), libc::SIGKILL) };
        if result == 0 {
            return;
        }
    }
    if let Err(error) = child.kill().await {
        tracing::warn!("timed-out hook process could not be killed: {error}");
    }
}

#[cfg(windows)]
async fn terminate_process_tree(child: &mut tokio::process::Child, process_id: Option<u32>) {
    if let Some(process_id) = process_id {
        // `taskkill /T` terminates descendants as well as the shell. Arguments are passed
        // directly (without a shell), and the pid is numeric, so no command injection occurs.
        if let Some(system_root) = std::env::var_os("SystemRoot") {
            let taskkill_path = std::path::PathBuf::from(system_root)
                .join("System32")
                .join("taskkill.exe");
            let mut taskkill = tokio::process::Command::new(taskkill_path);
            taskkill
                .args(["/PID", &process_id.to_string(), "/T", "/F"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true);
            if let Ok(Ok(status)) =
                tokio::time::timeout(Duration::from_secs(5), taskkill.status()).await
            {
                if status.success() {
                    return;
                }
            }
        }
    }
    if let Err(error) = child.kill().await {
        tracing::warn!("timed-out hook process could not be killed: {error}");
    }
}

#[cfg(not(any(unix, windows)))]
async fn terminate_process_tree(child: &mut tokio::process::Child, _process_id: Option<u32>) {
    if let Err(error) = child.kill().await {
        tracing::warn!("timed-out hook process could not be killed: {error}");
    }
}

#[cfg(windows)]
fn shell_command(command: &str) -> tokio::process::Command {
    // `cmd /C` は自前でコマンドラインを再パースするため、`Command::arg` の
    // CreateProcess 向けエスケープ (`"` → `\"`) を経由すると `command` 内の
    // `"..."` がそのまま `\"...\"` として cmd に渡ってしまい壊れる
    // (hooks.toml の `command = "python scripts/convert.py \"$SYNERGOS_FILE\""`
    // のような引用符入りコマンドが動かなくなる)。`raw_arg` で無加工のまま渡す。
    let mut cmd = tokio::process::Command::new("cmd");
    cmd.arg("/C");
    cmd.raw_arg(command);
    cmd
}

#[cfg(not(windows))]
fn shell_command(command: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c").arg(command);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;
    use synergos_net::config::HookDef;

    fn runner(hooks: Vec<HookDef>, allow_project_hooks: bool) -> HookRunner {
        HookRunner::new(HooksConfig {
            allow_project_hooks,
            hooks,
        })
    }

    fn shell_true() -> String {
        if cfg!(windows) {
            "exit 0".into()
        } else {
            "true".into()
        }
    }

    fn shell_false() -> String {
        if cfg!(windows) {
            "exit 1".into()
        } else {
            "false".into()
        }
    }

    #[tokio::test]
    async fn pre_publish_success_returns_ok() {
        let dir = tempfile::tempdir().unwrap();
        let r = runner(
            vec![HookDef {
                event: "pre-publish".into(),
                command: shell_true(),
                r#match: vec![],
                timeout_sec: 5,
            }],
            false,
        );
        assert!(r
            .run_pre_publish(dir.path(), "proj", "a.txt")
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn pre_publish_nonzero_exit_is_err() {
        let dir = tempfile::tempdir().unwrap();
        let r = runner(
            vec![HookDef {
                event: "pre-publish".into(),
                command: shell_false(),
                r#match: vec![],
                timeout_sec: 5,
            }],
            false,
        );
        assert!(r
            .run_pre_publish(dir.path(), "proj", "a.txt")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn pre_publish_timeout_is_err() {
        let dir = tempfile::tempdir().unwrap();
        let sleep_cmd = if cfg!(windows) {
            "ping -n 5 127.0.0.1 >NUL".to_string()
        } else {
            "sleep 5".to_string()
        };
        let r = runner(
            vec![HookDef {
                event: "pre-publish".into(),
                command: sleep_cmd,
                r#match: vec![],
                timeout_sec: 1,
            }],
            false,
        );
        let err = r
            .run_pre_publish(dir.path(), "proj", "a.txt")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("timed out"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_kills_descendant_processes() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("descendant-finished");
        let command = format!(
            "(sleep 2; printf leaked > \"{}\") & sleep 5",
            marker.display()
        );
        let r = runner(
            vec![HookDef {
                event: "pre-publish".into(),
                command,
                r#match: vec![],
                timeout_sec: 1,
            }],
            false,
        );

        assert!(r
            .run_pre_publish(dir.path(), "proj", "a.txt")
            .await
            .is_err());
        tokio::time::sleep(Duration::from_millis(2500)).await;
        assert!(
            !marker.exists(),
            "a descendant survived after its hook timed out"
        );
    }

    #[tokio::test]
    async fn pre_publish_skips_non_matching_files() {
        let dir = tempfile::tempdir().unwrap();
        let r = runner(
            vec![HookDef {
                event: "pre-publish".into(),
                command: shell_false(),
                r#match: vec!["assets/**/*.png".into()],
                timeout_sec: 5,
            }],
            false,
        );
        // a.txt doesn't match assets/**/*.png, so the failing hook never runs.
        assert!(r
            .run_pre_publish(dir.path(), "proj", "a.txt")
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn project_hooks_ignored_when_opt_in_disabled() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::create_dir_all(dir.path().join(".synergos"))
            .await
            .unwrap();
        tokio::fs::write(
            dir.path().join(".synergos/hooks.toml"),
            format!(
                "[[hook]]\nevent = \"pre-publish\"\ncommand = \"{}\"\n",
                shell_false()
            ),
        )
        .await
        .unwrap();
        let r = runner(vec![], false);
        // allow_project_hooks = false → project hook must not run (and thus not fail).
        assert!(r
            .run_pre_publish(dir.path(), "proj", "a.txt")
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn project_hooks_run_when_opt_in_enabled() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::create_dir_all(dir.path().join(".synergos"))
            .await
            .unwrap();
        tokio::fs::write(
            dir.path().join(".synergos/hooks.toml"),
            format!(
                "[[hook]]\nevent = \"pre-publish\"\ncommand = \"{}\"\n",
                shell_false()
            ),
        )
        .await
        .unwrap();
        let r = runner(vec![], true);
        assert!(r
            .run_pre_publish(dir.path(), "proj", "a.txt")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn effective_hooks_lists_disabled_project_hooks() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::create_dir_all(dir.path().join(".synergos"))
            .await
            .unwrap();
        tokio::fs::write(
            dir.path().join(".synergos/hooks.toml"),
            "[[hook]]\nevent = \"post-receive\"\ncommand = \"true\"\n",
        )
        .await
        .unwrap();
        let r = runner(vec![], false);
        let effective = r.effective_hooks(dir.path()).await.unwrap();
        assert_eq!(effective.len(), 1);
        assert_eq!(effective[0].source, HookSource::Project);
        assert!(effective[0].disabled_by_opt_in);
    }

    #[tokio::test]
    async fn run_manual_executes_matching_hooks_and_reports_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let r = runner(
            vec![HookDef {
                event: "post-receive".into(),
                command: shell_true(),
                r#match: vec![],
                timeout_sec: 5,
            }],
            false,
        );
        let outcomes = r
            .run_manual(dir.path(), HookEvent::PostReceive, "proj", "a.txt")
            .await
            .unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].status, HookStatus::Success);
    }
}
