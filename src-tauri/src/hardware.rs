use serde::Serialize;
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::io::AsyncReadExt;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Gpu {
    name: String,
    uuid: String,
    memory_used_mib: Option<u64>,
    memory_total_mib: Option<u64>,
    utilization_percent: Option<u8>,
    driver_version: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hardware {
    logical_cpus: usize,
    memory_total_bytes: Option<u64>,
    memory_available_bytes: Option<u64>,
    gpus: Vec<Gpu>,
    gpu_status: String,
    sampled_at: i64,
}

impl Hardware {
    pub fn free_vram_bytes(&self) -> Option<u64> {
        let gpu = self.gpus.first()?;
        Some(gpu.memory_total_mib?.saturating_sub(gpu.memory_used_mib?) * 1024 * 1024)
    }
}
fn optional_number(value: &str) -> Result<Option<u64>, String> {
    if matches!(value, "N/A" | "[N/A]" | "[Not Supported]" | "Not Supported") {
        return Ok(None);
    }
    value
        .parse()
        .map(Some)
        .map_err(|_| "GPU driver returned an invalid numeric value.".into())
}
fn parse_gpus(text: &str) -> Result<Vec<Gpu>, String> {
    let mut gpus = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        if gpus.len() >= 32 {
            return Err("GPU query returned too many devices.".into());
        }
        let columns: Vec<_> = line.split(',').map(str::trim).collect();
        if columns.len() != 6 || columns[0].is_empty() || !columns[1].starts_with("GPU-") {
            return Err("GPU driver returned an unexpected device record.".into());
        }
        let used = optional_number(columns[2])?;
        let total = optional_number(columns[3])?;
        let utilization = optional_number(columns[4])?;
        if utilization.is_some_and(|value| value > 100)
            || total.zip(used).is_some_and(|(total, used)| used > total)
        {
            return Err("GPU driver returned inconsistent readings.".into());
        }
        gpus.push(Gpu {
            name: columns[0].into(),
            uuid: columns[1].into(),
            memory_used_mib: used,
            memory_total_mib: total,
            utilization_percent: utilization.map(|value| value as u8),
            driver_version: columns[5].into(),
        });
    }
    Ok(gpus)
}
fn nvidia_smi() -> Option<PathBuf> {
    let mut paths = Vec::new();
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        paths.push(PathBuf::from(root).join("System32/nvidia-smi.exe"));
    }
    if let Some(path) = std::env::var_os("PATH") {
        paths.extend(
            std::env::split_paths(&path)
                .filter(|path| path.is_absolute())
                .map(|path| {
                    path.join(if cfg!(windows) {
                        "nvidia-smi.exe"
                    } else {
                        "nvidia-smi"
                    })
                }),
        );
    }
    paths.into_iter().find(|path| path.is_file())
}
async fn query_gpu() -> Result<Vec<Gpu>, String> {
    let program = nvidia_smi()
        .ok_or("NVIDIA telemetry is unavailable. This does not rule out other GPU backends.")?;
    let mut command = tokio::process::Command::new(program);
    command
        .args([
            "--query-gpu=name,uuid,memory.used,memory.total,utilization.gpu,driver_version",
            "--format=csv,noheader,nounits",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command
        .spawn()
        .map_err(|_| "Could not start the NVIDIA telemetry query.")?;
    let output = child
        .stdout
        .take()
        .ok_or("GPU query output pipe is unavailable.")?;
    let mut bytes = Vec::new();
    let status = tokio::time::timeout(Duration::from_secs(4), async {
        output
            .take(65_537)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| "GPU query output failed.")?;
        if bytes.len() > 65_536 {
            return Err("GPU query exceeded its output limit.");
        }
        child.wait().await.map_err(|_| "GPU query process failed.")
    })
    .await
    .map_err(|_| "GPU telemetry query timed out.")??;
    if !status.success() {
        return Err("NVIDIA driver telemetry is unavailable or returned an error.".into());
    }
    parse_gpus(&String::from_utf8(bytes).map_err(|_| "GPU query returned invalid text.")?)
}
fn memory() -> (Option<u64>, Option<u64>) {
    #[cfg(windows)]
    {
        use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        // The initialized structure has the exact size required by the Windows API.
        if unsafe { GlobalMemoryStatusEx(&mut status) }.is_ok() {
            return (Some(status.ullTotalPhys), Some(status.ullAvailPhys));
        }
    }
    (None, None)
}
#[tauri::command]
pub async fn hardware_status() -> Hardware {
    let (memory_total_bytes, memory_available_bytes) = memory();
    let (gpus, gpu_status) = match query_gpu().await {
        Ok(gpus) => (gpus, "NVIDIA driver telemetry".into()),
        Err(error) => (Vec::new(), error),
    };
    Hardware {
        logical_cpus: std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(1),
        memory_total_bytes,
        memory_available_bytes,
        gpus,
        gpu_status,
        sampled_at: crate::store::now(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_multiple_devices_and_preserves_unavailable_readings() {
        let devices=parse_gpus("RTX 2050, GPU-test, 2047, 4096, 37, 581.42\r\nOther, GPU-other, N/A, 8192, [Not Supported], 581.42\n").unwrap();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].utilization_percent, Some(37));
        assert_eq!(devices[1].memory_used_mib, None);
        assert_eq!(devices[1].utilization_percent, None);
        let hardware = Hardware {
            logical_cpus: 8,
            memory_total_bytes: None,
            memory_available_bytes: None,
            gpus: devices,
            gpu_status: "ok".into(),
            sampled_at: 0,
        };
        assert_eq!(hardware.free_vram_bytes(), Some((4096 - 2047) * 1024 * 1024));
    }
    #[test]
    fn rejects_invalid_or_inconsistent_driver_output() {
        for line in [
            "bad",
            "GPU, GPU-x, -1, 4096, 0, driver",
            "GPU, GPU-x, 5000, 4096, 0, driver",
            "GPU, GPU-x, 0, 4096, 101, driver",
        ] {
            assert!(parse_gpus(line).is_err());
        }
    }
}
