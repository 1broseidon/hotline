//! Advisory capacity, never a reservation or a live container resize.
use super::runtime::{self, BinSearch, Runtime};
use crate::contract::{ComputerCapacity, ComputerCapacitySource, RuntimeReport, RuntimeState};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const CACHE_FOR: Duration = Duration::from_secs(60);
const DEFAULT_MEMORY_BYTES: u64 = 8 * 1024 * 1024 * 1024;

#[derive(Default)]
pub(super) struct Cache {
    // Keep the lock across discovery: clones and concurrent requests share one
    // probe. At most four preference entries, including automatic selection.
    entries: Mutex<HashMap<Option<Runtime>, (Instant, ComputerCapacity)>>,
}

impl Cache {
    pub(super) async fn get(
        &self,
        prefer: Option<Runtime>,
        probe: &impl Probe,
    ) -> ComputerCapacity {
        let mut entries = self.entries.lock().await;
        if let Some((at, answer)) = entries.get(&prefer)
            && at.elapsed() < CACHE_FOR
        {
            return answer.clone();
        }
        let answer = discover(prefer, probe).await;
        entries.insert(prefer, (Instant::now(), answer.clone()));
        answer
    }
}

#[async_trait::async_trait]
pub(super) trait Probe: Sync {
    async fn reports(&self) -> Vec<RuntimeReport>;
    async fn info(&self, runtime: Runtime) -> Option<String>;
    fn host(&self) -> Option<(u32, u64)>;
}

pub(super) struct SystemProbe<'a>(pub &'a BinSearch);

#[async_trait::async_trait]
impl Probe for SystemProbe<'_> {
    async fn reports(&self) -> Vec<RuntimeReport> {
        runtime::detect_with(self.0).await
    }

    async fn info(&self, runtime: Runtime) -> Option<String> {
        runtime::capacity_info(runtime, self.0).await
    }

    fn host(&self) -> Option<(u32, u64)> {
        host_capacity()
    }
}

async fn discover(prefer: Option<Runtime>, probe: &impl Probe) -> ComputerCapacity {
    let reports = probe.reports().await;
    let installed = |report: &&RuntimeReport| {
        !matches!(
            report.state,
            RuntimeState::NotInstalled | RuntimeState::Unsupported
        )
    };
    // Detection already ranks ready/rootless runtimes as lifecycle selection
    // does. An explicit installed choice wins even with a stopped daemon. If
    // that choice is absent, report the best installed alternative; null means
    // there really is no installed runtime, not that a daemon failed a probe.
    let selected = prefer
        .and_then(|want| {
            reports
                .iter()
                .filter(installed)
                .find(|r| r.runtime == want.wire())
        })
        .or_else(|| reports.iter().find(installed));
    let runtime = selected.map(|report| Runtime::from_wire(report.runtime));
    if let Some(runtime) = runtime
        && runtime != Runtime::AppleContainer
        && let Some(info) = probe.info(runtime).await
        && let Some((cpus, memory_bytes)) = parse_info(runtime, &info)
    {
        return ComputerCapacity {
            runtime: Some(runtime.wire()),
            cpus,
            memory_bytes,
            source: ComputerCapacitySource::Runtime,
        };
    }
    let (cpus, memory_bytes, source) = match probe
        .host()
        .filter(|(cpus, memory)| *cpus > 0 && *memory > 0)
    {
        Some((cpus, memory)) => (cpus, memory, ComputerCapacitySource::Host),
        None => (4, DEFAULT_MEMORY_BYTES, ComputerCapacitySource::Default),
    };
    ComputerCapacity {
        runtime: runtime.map(Runtime::wire),
        cpus,
        memory_bytes,
        source,
    }
}

fn parse_info(runtime: Runtime, info: &str) -> Option<(u32, u64)> {
    let info: serde_json::Value = serde_json::from_str(info).ok()?;
    let (cpus, memory) = match runtime {
        Runtime::Docker => (info.get("NCPU")?, info.get("MemTotal")?),
        Runtime::Podman => {
            let host = info.get("host")?;
            (host.get("cpus")?, host.get("memTotal")?)
        }
        Runtime::AppleContainer => return None,
    };
    let cpus = u32::try_from(cpus.as_u64()?).ok()?;
    let memory = memory.as_u64()?;
    (cpus > 0 && memory > 0).then_some((cpus, memory))
}

fn host_capacity() -> Option<(u32, u64)> {
    let cpus = host_cpus()?;
    let memory = host_memory()?;
    (cpus > 0 && memory > 0).then_some((cpus, memory))
}

#[cfg(unix)]
fn host_cpus() -> Option<u32> {
    // Logical online host CPUs, rather than the affinity of the desk process.
    // SAFETY: sysconf takes no pointers and has no preconditions.
    u32::try_from(unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) }).ok()
}

#[cfg(not(unix))]
fn host_cpus() -> Option<u32> {
    u32::try_from(std::thread::available_parallelism().ok()?.get()).ok()
}

#[cfg(target_os = "linux")]
fn host_memory() -> Option<u64> {
    // SAFETY: these sysconf selectors require no pointers or initialization.
    let pages = u64::try_from(unsafe { libc::sysconf(libc::_SC_PHYS_PAGES) }).ok()?;
    let page_size = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).ok()?;
    pages.checked_mul(page_size)
}

#[cfg(target_os = "macos")]
fn host_memory() -> Option<u64> {
    let mut bytes: u64 = 0;
    let mut len = std::mem::size_of_val(&bytes);
    // SAFETY: hw.memsize returns a uint64_t; both output pointers are valid
    // for their supplied lengths. A null newp performs a read, never a write.
    let result = unsafe {
        libc::sysctlbyname(
            c"hw.memsize".as_ptr(),
            (&mut bytes as *mut u64).cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    (result == 0 && len == std::mem::size_of_val(&bytes)).then_some(bytes)
}

#[cfg(windows)]
fn host_memory() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    // SAFETY: MEMORYSTATUSEX is a plain C output structure; its size must be
    // initialized before calling GlobalMemoryStatusEx with a valid pointer.
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    (unsafe { GlobalMemoryStatusEx(&mut status) } != 0).then_some(status.ullTotalPhys)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn host_memory() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fake {
        reports: Vec<RuntimeReport>,
        info: Option<String>,
        host: Option<(u32, u64)>,
        calls: AtomicUsize,
        infos: AtomicUsize,
    }
    #[async_trait::async_trait]
    impl Probe for Fake {
        async fn reports(&self) -> Vec<RuntimeReport> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::task::yield_now().await;
            self.reports.clone()
        }
        async fn info(&self, _: Runtime) -> Option<String> {
            self.infos.fetch_add(1, Ordering::SeqCst);
            self.info.clone()
        }
        fn host(&self) -> Option<(u32, u64)> {
            self.host
        }
    }
    fn report(runtime: Runtime, state: RuntimeState) -> RuntimeReport {
        RuntimeReport {
            runtime: runtime.wire(),
            state,
            detail: None,
            rootless: false,
        }
    }
    fn fake(reports: Vec<RuntimeReport>, info: Option<&str>) -> Fake {
        Fake {
            reports,
            info: info.map(str::to_owned),
            host: Some((12, 32 << 30)),
            calls: AtomicUsize::new(0),
            infos: AtomicUsize::new(0),
        }
    }

    #[tokio::test]
    async fn runtime_capacity_precedes_host_for_docker_and_podman() {
        for (runtime, json) in [
            (Runtime::Docker, r#"{"NCPU":6,"MemTotal":8589934592}"#),
            (
                Runtime::Podman,
                r#"{"host":{"cpus":6,"memTotal":8589934592}}"#,
            ),
        ] {
            let answer = discover(
                None,
                &fake(vec![report(runtime, RuntimeState::Ready)], Some(json)),
            )
            .await;
            assert_eq!(answer.runtime, Some(runtime.wire()));
            assert_eq!((answer.cpus, answer.memory_bytes), (6, 8 << 30));
            assert_eq!(answer.source, ComputerCapacitySource::Runtime);
        }
    }

    #[tokio::test]
    async fn malformed_zero_missing_and_failed_info_use_host_but_keep_runtime() {
        for json in [
            None,
            Some("oops"),
            Some("{}"),
            Some(r#"{"NCPU":0,"MemTotal":8}"#),
            Some(r#"{"NCPU":4,"MemTotal":0}"#),
            Some(r#"{"NCPU":4294967296,"MemTotal":8}"#),
        ] {
            let answer = discover(
                None,
                &fake(
                    vec![report(Runtime::Docker, RuntimeState::NotRunning)],
                    json,
                ),
            )
            .await;
            assert_eq!(answer.runtime, Some(Runtime::Docker.wire()));
            assert_eq!((answer.cpus, answer.memory_bytes), (12, 32 << 30));
            assert_eq!(answer.source, ComputerCapacitySource::Host);
        }
    }

    #[tokio::test]
    async fn explicit_installed_preference_wins_even_when_daemon_is_down() {
        let probe = fake(
            vec![
                report(Runtime::Podman, RuntimeState::Ready),
                report(Runtime::Docker, RuntimeState::NotRunning),
            ],
            None,
        );
        assert_eq!(
            discover(None, &probe).await.runtime,
            Some(Runtime::Podman.wire())
        );
        let answer = discover(Some(Runtime::Docker), &probe).await;
        assert_eq!(answer.runtime, Some(Runtime::Docker.wire()));
        assert_eq!(answer.source, ComputerCapacitySource::Host);
    }

    #[tokio::test]
    async fn absent_preference_still_reports_an_installed_runtime() {
        let probe = fake(
            vec![
                report(Runtime::Docker, RuntimeState::NotInstalled),
                report(Runtime::Podman, RuntimeState::Failed),
            ],
            None,
        );
        assert_eq!(
            discover(Some(Runtime::Docker), &probe).await.runtime,
            Some(Runtime::Podman.wire())
        );
    }

    #[tokio::test]
    async fn apple_uses_host_without_an_info_subprocess() {
        let probe = fake(
            vec![report(Runtime::AppleContainer, RuntimeState::Ready)],
            Some("unused"),
        );
        let answer = discover(None, &probe).await;
        assert_eq!(answer.runtime, Some(Runtime::AppleContainer.wire()));
        assert_eq!(answer.source, ComputerCapacitySource::Host);
        assert_eq!(probe.infos.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn no_runtime_uses_host_then_conservative_default() {
        let mut probe = fake(
            vec![
                report(Runtime::Docker, RuntimeState::NotInstalled),
                report(Runtime::AppleContainer, RuntimeState::Unsupported),
            ],
            None,
        );
        let answer = discover(None, &probe).await;
        assert_eq!(answer.runtime, None);
        assert_eq!(answer.source, ComputerCapacitySource::Host);
        for host in [None, Some((0, 8)), Some((4, 0))] {
            probe.host = host;
            let answer = discover(None, &probe).await;
            assert_eq!(answer.runtime, None);
            assert_eq!((answer.cpus, answer.memory_bytes), (4, 8 << 30));
            assert_eq!(answer.source, ComputerCapacitySource::Default);
        }
    }

    #[tokio::test]
    async fn cache_coalesces_requests_keys_preferences_and_expires() {
        let cache = Cache::default();
        let probe = fake(vec![], None);
        tokio::join!(cache.get(None, &probe), cache.get(None, &probe));
        cache.get(None, &probe).await;
        assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
        cache.get(Some(Runtime::Docker), &probe).await;
        assert_eq!(probe.calls.load(Ordering::SeqCst), 2);
        cache.entries.lock().await.get_mut(&None).unwrap().0 = Instant::now() - CACHE_FOR;
        cache.get(None, &probe).await;
        assert_eq!(probe.calls.load(Ordering::SeqCst), 3);
    }
}
