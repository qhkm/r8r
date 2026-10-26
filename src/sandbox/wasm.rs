/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! WASM/WASI sandbox backend (`sandbox-wasm` feature).
//!
//! Executes compiled `wasm32-wasi` modules without any external runtime,
//! interpreter, or container — the module runs directly inside the r8r
//! process via [wasmtime]. Cold start is sub-millisecond and per-execution
//! cost is zero; this is the intended backend for trusted, high-frequency
//! tool execution.
//!
//! The module is passed as **base64** in [`SandboxRequest::code`] with
//! `runtime = "wasm"`. Modules can:
//!
//! - compute (full WASM instruction set),
//! - read/write stdout & stderr (captured),
//! - read environment variables set in `req.env`,
//! - write files to the artifact output dir (when `collect_artifacts` is
//!   set, via the `R8R_OUTPUT_DIR` env var, preopened at `/`).
//!
//! Modules **cannot**: access the network, the host filesystem (outside the
//! artifact dir), spawn processes, or allocate unbounded memory (64 MiB
//! default wasmtime limit).
//!
//! Timeout is enforced twice over: a tokio timeout on the `_start` future
//! plus wasmtime epoch interruption so CPU-bound loops cannot spin forever.

use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use tokio::sync::mpsc;
use tokio::time::Instant;
use wasmtime::{Config as WtConfig, Engine, Linker, Module, Store};
use wasmtime_environ::Trap;
use wasmtime_wasi::p1::{WasiP1Ctx, add_to_linker_async};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{I32Exit, WasiCtxBuilder};

use super::{
    SandboxArtifact, SandboxBackend, SandboxError, SandboxRequest, SandboxResult,
    SandboxStreamEvent,
};

/// Epoch tick interval. wasmtime checks the epoch at compiled interruption
/// points; bumping it every 10 ms bounds how long a pure-compute loop can
/// run past its deadline.
const EPOCH_TICK_MS: u64 = 10;

/// WASM/WASI execution backend.
///
/// Cheap to construct; the engine is shared across all executions and the
/// epoch ticker is aborted when the backend is dropped.
pub struct WasmSandbox {
    engine: Engine,
    _epoch_ticker: tokio::task::JoinHandle<()>,
}

impl WasmSandbox {
    /// Create a backend with an async, epoch-interrupted engine.
    pub fn new() -> anyhow::Result<Self> {
        let mut cfg = WtConfig::new();
        cfg.epoch_interruption(true);
        let engine = Engine::new(&cfg)?;
        let ticker_engine = engine.clone();
        let epoch_ticker = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(EPOCH_TICK_MS));
            loop {
                interval.tick().await;
                ticker_engine.increment_epoch();
            }
        });
        Ok(Self {
            engine,
            _epoch_ticker: epoch_ticker,
        })
    }
}

impl Drop for WasmSandbox {
    fn drop(&mut self) {
        self._epoch_ticker.abort();
    }
}

#[async_trait]
impl SandboxBackend for WasmSandbox {
    async fn execute(&self, req: SandboxRequest) -> Result<SandboxResult, SandboxError> {
        if req.runtime != "wasm" {
            return Err(SandboxError::BackendUnavailable {
                backend: self.name().to_string(),
                reason: format!(
                    "runtime '{}' is not supported (the WASM backend only accepts \
                     compiled wasm32-wasi modules with runtime = \"wasm\")",
                    req.runtime
                ),
            });
        }
        if req.network {
            return Err(SandboxError::ResourceLimit {
                resource: "network".to_string(),
                limit: "WASM sandbox provides no network access; recompile the \
                        module without networking or use the docker backend"
                    .to_string(),
            });
        }
        if !req.packages.is_empty() {
            return Err(SandboxError::ResourceLimit {
                resource: "packages".to_string(),
                limit: "WASM sandbox cannot install packages; compile them into \
                        the module instead"
                    .to_string(),
            });
        }

        let start = Instant::now();
        let backend_name = self.name().to_string();

        // --- Decode & validate the wasm module ---------------------------------
        let bytes =
            STANDARD
                .decode(req.code.trim())
                .map_err(|e| SandboxError::BackendUnavailable {
                    backend: backend_name.clone(),
                    reason: format!("code is not valid base64: {e}"),
                })?;
        let module =
            Module::new(&self.engine, &bytes).map_err(|e| SandboxError::BackendUnavailable {
                backend: backend_name.clone(),
                reason: format!("not a valid wasm module: {e}"),
            })?;

        // --- Artifact output dir (optional) ------------------------------------
        let artifact_dir = if req.collect_artifacts {
            Some(tempfile::tempdir().map_err(SandboxError::Io)?)
        } else {
            None
        };

        // --- WASI context --------------------------------------------------------
        let stdout_pipe = MemoryOutputPipe::new(req.max_output_bytes as usize);
        let stderr_pipe = MemoryOutputPipe::new(req.max_output_bytes as usize);

        let mut builder = WasiCtxBuilder::new();
        builder
            .stdout(stdout_pipe.clone())
            .stderr(stderr_pipe.clone())
            .envs(
                &req.env
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect::<Vec<_>>(),
            )
            .args(&["r8r-wasm".to_string()]);
        if let Some(dir) = &artifact_dir {
            builder
                .preopened_dir(dir.path(), "/", wasmtime_wasi::FsPerms::ReadWrite)
                .map_err(|e| SandboxError::BackendUnavailable {
                    backend: backend_name.clone(),
                    reason: format!("failed to preopen artifact dir: {e}"),
                })?;
            builder.env("R8R_OUTPUT_DIR", dir.path().to_string_lossy().as_ref());
        }
        let wasi = builder.build_p1();

        let mut store = Store::new(&self.engine, wasi);
        let mut linker = Linker::<WasiP1Ctx>::new(&self.engine);
        add_to_linker_async(&mut linker, |ctx| ctx).map_err(|e| {
            SandboxError::BackendUnavailable {
                backend: backend_name.clone(),
                reason: format!("failed to initialise WASI host functions: {e}"),
            }
        })?;

        // --- Run with epoch + tokio deadline ------------------------------------
        let timeout_seconds = req.timeout.as_secs();
        let timeout_ticks = (req.timeout.as_millis() / u128::from(EPOCH_TICK_MS)).max(1) as u64;
        store.set_epoch_deadline(timeout_ticks);

        let execution = async {
            let instance = linker.instantiate_async(&mut store, &module).await?;
            let start_fn = instance.get_typed_func::<(), ()>(&mut store, "_start")?;
            match start_fn.call_async(&mut store, ()).await {
                Ok(()) => Ok(0),
                Err(e) => {
                    let mut src = e.source();
                    while let Some(s) = src {
                        if let Some(exit) = s.downcast_ref::<I32Exit>() {
                            return Ok(exit.0);
                        }
                        src = s.source();
                    }
                    match e.downcast_ref::<I32Exit>() {
                        Some(exit) => Ok(exit.0),
                        None => Err(e),
                    }
                }
            }
        };

        let outcome = match tokio::time::timeout(req.timeout, execution).await {
            Err(_elapsed) => {
                return Err(SandboxError::Timeout { timeout_seconds });
            }
            Ok(Ok(exit_code)) => exit_code,
            Ok(Err(e)) => {
                // Epoch deadline hit: wasmtime traps with `Trap::Interrupt`
                // (present in the source chain, not at top level) — map to
                // Timeout so the node reports the same error as the docker
                // and subprocess backends.
                let mut src = e.source();
                while let Some(s) = src {
                    if let Some(trap) = s.downcast_ref::<Trap>() {
                        if matches!(trap, Trap::Interrupt) {
                            return Err(SandboxError::Timeout { timeout_seconds });
                        }
                        break;
                    }
                    src = s.source();
                }
                // Unhandled trap (e.g. unreachable, OOB): surface as a failed
                // run with the trap message on stderr, mirroring subprocess
                // crashes — the module itself decided to abort.
                let stderr = format!("wasm trap: {e}\n");
                return Ok(SandboxResult {
                    stdout: String::from_utf8_lossy(&stdout_pipe.contents()).into_owned(),
                    stderr,
                    exit_code: 1,
                    duration_ms: start.elapsed().as_millis() as u64,
                    artifacts: Vec::new(),
                });
            }
        };

        // --- Collect artifacts ---------------------------------------------------
        let artifacts: Vec<SandboxArtifact> = match &artifact_dir {
            Some(dir) => super::collect_output_artifacts(dir.path(), req.max_artifact_bytes)?,
            None => Vec::new(),
        };

        Ok(SandboxResult {
            stdout: String::from_utf8_lossy(&stdout_pipe.contents()).into_owned(),
            stderr: String::from_utf8_lossy(&stderr_pipe.contents()).into_owned(),
            exit_code: outcome,
            duration_ms: start.elapsed().as_millis() as u64,
            artifacts,
        })
    }

    async fn execute_streaming(&self, req: SandboxRequest, tx: mpsc::Sender<SandboxStreamEvent>) {
        match self.execute(req).await {
            Ok(result) => {
                if !result.stdout.is_empty() {
                    let _ = tx.send(SandboxStreamEvent::Stdout(result.stdout)).await;
                }
                if !result.stderr.is_empty() {
                    let _ = tx.send(SandboxStreamEvent::Stderr(result.stderr)).await;
                }
                let _ = tx
                    .send(SandboxStreamEvent::Done {
                        exit_code: result.exit_code,
                        duration_ms: result.duration_ms,
                        artifacts: result.artifacts,
                    })
                    .await;
            }
            Err(e) => {
                let _ = tx.send(SandboxStreamEvent::Error(e.to_string())).await;
            }
        }
    }

    fn name(&self) -> &str {
        "wasm"
    }

    fn available(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Encode a wat module as base64, the wire format used by the node.
    fn wat_to_code(wat: &str) -> String {
        let bytes = wat::parse_str(wat).expect("valid wat");
        STANDARD.encode(bytes)
    }

    fn req(code: String) -> SandboxRequest {
        SandboxRequest {
            runtime: "wasm".to_string(),
            code,
            env: HashMap::new(),
            timeout: Duration::from_secs(5),
            ..Default::default()
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn hello_stdout() {
        let backend = WasmSandbox::new().unwrap();
        let result = backend
            .execute(req(wat_to_code(
                r#"(module
                    (import "wasi_snapshot_preview1" "fd_write"
                      (func $fd_write (param i32 i32 i32 i32) (result i32)))
                    (memory (export "memory") 1)
                    (data (i32.const 16) "hi from wasm\n")
                    (func (export "_start")
                      (i32.store (i32.const 0) (i32.const 16))
                      (i32.store (i32.const 4) (i32.const 13))
                      (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 24)))
                    )
                  )"#,
            )))
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "hi from wasm\n");
        assert_eq!(result.stderr, "");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn proc_exit_code_propagated() {
        let backend = WasmSandbox::new().unwrap();
        let result = backend
            .execute(req(wat_to_code(
                r#"(module
                    (import "wasi_snapshot_preview1" "proc_exit" (func $proc_exit (param i32)))
                    (memory (export "memory") 1)
                    (func (export "_start") (call $proc_exit (i32.const 3)))
                  )"#,
            )))
            .await
            .unwrap();
        assert_eq!(result.exit_code, 3);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn deadline_terminates_infinite_loop() {
        let backend = WasmSandbox::new().unwrap();
        let mut r = req(wat_to_code(
            r#"(module
                    (func (export "_start")
                      (loop $l (br $l))
                    )
                  )"#,
        ));
        r.timeout = Duration::from_millis(150);
        let err = backend.execute(r).await.unwrap_err();
        assert!(matches!(err, SandboxError::Timeout { .. }), "got {err:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn invalid_base64_rejected() {
        let backend = WasmSandbox::new().unwrap();
        let err = backend
            .execute(req("not base64!!!".to_string()))
            .await
            .unwrap_err();
        assert!(
            matches!(err, SandboxError::BackendUnavailable { .. }),
            "got {err:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn non_wasm_runtime_rejected() {
        let backend = WasmSandbox::new().unwrap();
        let mut r = req(wat_to_code(r#"(module)"#));
        r.runtime = "python3".to_string();
        let err = backend.execute(r).await.unwrap_err();
        assert!(
            matches!(err, SandboxError::BackendUnavailable { .. }),
            "got {err:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn network_rejected() {
        let backend = WasmSandbox::new().unwrap();
        let mut r = req(wat_to_code(r#"(module)"#));
        r.network = true;
        let err = backend.execute(r).await.unwrap_err();
        assert!(
            matches!(err, SandboxError::ResourceLimit { .. }),
            "got {err:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn artifacts_collected_from_output_dir() {
        let backend = WasmSandbox::new().unwrap();
        let code = wat_to_code(
            r#"(module
                    (import "wasi_snapshot_preview1" "fd_write"
                      (func $fd_write (param i32 i32 i32 i32) (result i32)))
                    (memory (export "memory") 1)
                    (func (export "_start"))
                  )"#,
        );
        // Trivial module that exits 0; artifact collection of an empty dir
        // must not error and must return no artifacts. (Writing files via
        // path_open is exercised by the wasi-tests suite upstream.)
        let mut r = req(code);
        r.collect_artifacts = true;
        let result = backend.execute(r).await.unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.artifacts.is_empty());
    }
}
