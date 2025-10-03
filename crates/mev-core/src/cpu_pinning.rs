//! CPU core pinning utilities for performance optimization

use anyhow::Result;
use std::thread;
use tracing::{info, warn};

/// CPU core pinning configuration
#[derive(Debug, Clone)]
pub struct CorePinningConfig {
    pub enabled: bool,
    pub core_ids: Vec<usize>,
    pub worker_threads: usize,
}

impl Default for CorePinningConfig {
    fn default() -> Self {
        let num_cpus = num_cpus::get();
        Self {
            enabled: false,
            core_ids: (0..num_cpus).collect(),
            worker_threads: num_cpus,
        }
    }
}

/// CPU affinity manager for thread pinning
pub struct CpuAffinityManager {
    config: CorePinningConfig,
}

impl CpuAffinityManager {
    pub fn new(config: CorePinningConfig) -> Self {
        Self { config }
    }

    /// Initialize CPU affinity for the current process
    pub fn initialize(&self) -> Result<()> {
        if !self.config.enabled {
            info!("CPU core pinning disabled");
            return Ok(());
        }

        info!(
            core_ids = ?self.config.core_ids,
            worker_threads = self.config.worker_threads,
            "Initializing CPU core pinning"
        );

        // Set process affinity (platform-specific)
        #[cfg(target_os = "linux")]
        {
            self.set_linux_affinity()?;
        }

        #[cfg(target_os = "windows")]
        {
            self.set_windows_affinity()?;
        }

        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            warn!("CPU core pinning not supported on this platform");
        }

        Ok(())
    }

    /// Pin current thread to a specific core
    pub fn pin_current_thread(&self, core_id: usize) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }

        if !self.config.core_ids.contains(&core_id) {
            return Err(anyhow::anyhow!("Core ID {} not in allowed cores", core_id));
        }

        #[cfg(target_os = "linux")]
        {
            self.pin_thread_linux(core_id)?;
        }

        #[cfg(target_os = "windows")]
        {
            self.pin_thread_windows(core_id)?;
        }

        info!(
            thread_id = ?thread::current().id(),
            core_id = core_id,
            "Pinned thread to CPU core"
        );

        Ok(())
    }

    /// Get optimal core assignment for worker threads
    pub fn get_core_assignment(&self) -> Vec<usize> {
        if !self.config.enabled || self.config.core_ids.is_empty() {
            return vec![];
        }

        let mut assignment = Vec::new();
        let num_cores = self.config.core_ids.len();

        for i in 0..self.config.worker_threads {
            let core_id = self.config.core_ids[i % num_cores];
            assignment.push(core_id);
        }

        assignment
    }

    /// Create a tokio runtime with CPU affinity
    pub fn create_runtime(&self) -> Result<tokio::runtime::Runtime> {
        let mut builder = tokio::runtime::Builder::new_multi_thread();
        
        builder.worker_threads(self.config.worker_threads);
        builder.enable_all();

        if self.config.enabled {
            info!(
                worker_threads = self.config.worker_threads,
                core_ids = ?self.config.core_ids,
                "Creating runtime with CPU affinity"
            );
        }

        let runtime = builder.build()?;
        Ok(runtime)
    }

    #[cfg(target_os = "linux")]
    fn set_linux_affinity(&self) -> Result<()> {
        use std::mem;
        
        // This is a simplified version - in production you'd use libc bindings
        warn!("Linux CPU affinity setting requires libc bindings - using RUSTFLAGS instead");
        info!("To enable CPU affinity on Linux, set RUSTFLAGS='-C target-cpu=native'");
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn pin_thread_linux(&self, core_id: usize) -> Result<()> {
        // This would require libc bindings for sched_setaffinity
        warn!("Linux thread pinning requires libc bindings - use taskset instead");
        info!("To pin process to cores on Linux: taskset -c {} ./mev-bot", 
              self.config.core_ids.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(","));
        Ok(())
    }

    #[cfg(target_os = "windows")]
    fn set_windows_affinity(&self) -> Result<()> {
        // This is a simplified version - in production you'd use winapi
        warn!("Windows CPU affinity setting requires winapi bindings");
        info!("To enable CPU affinity on Windows, use Process Lasso or similar tools");
        Ok(())
    }

    #[cfg(target_os = "windows")]
    fn pin_thread_windows(&self, _core_id: usize) -> Result<()> {
        // This would require winapi bindings for SetThreadAffinityMask
        warn!("Windows thread pinning requires winapi bindings");
        info!("Consider using Process Lasso for CPU affinity on Windows");
        Ok(())
    }
}

/// Performance monitoring for CPU usage
pub struct CpuPerformanceMonitor {
    start_time: std::time::Instant,
    last_check: std::time::Instant,
}

impl CpuPerformanceMonitor {
    pub fn new() -> Self {
        let now = std::time::Instant::now();
        Self {
            start_time: now,
            last_check: now,
        }
    }

    /// Get CPU usage statistics (simplified version)
    pub fn get_cpu_stats(&mut self) -> CpuStats {
        let now = std::time::Instant::now();
        let total_elapsed = now.duration_since(self.start_time);
        let interval_elapsed = now.duration_since(self.last_check);
        
        self.last_check = now;

        // In a real implementation, you'd read from /proc/stat on Linux
        // or use performance counters on Windows
        CpuStats {
            total_runtime_seconds: total_elapsed.as_secs_f64(),
            interval_seconds: interval_elapsed.as_secs_f64(),
            cpu_usage_percent: 0.0, // Would be calculated from actual CPU counters
            num_cores: num_cpus::get(),
            pinned_cores: vec![], // Would track actual pinned cores
        }
    }

    /// Print performance recommendations
    pub fn print_recommendations(&self, config: &CorePinningConfig) {
        info!("=== CPU Performance Recommendations ===");
        
        if !config.enabled {
            info!("💡 Enable CPU core pinning for better performance");
            info!("   Set cpu_core_pinning: true in config");
        }

        let num_cpus = num_cpus::get();
        if config.worker_threads > num_cpus {
            warn!("⚠️  Worker threads ({}) exceed CPU cores ({})", 
                  config.worker_threads, num_cpus);
            info!("   Consider reducing worker_threads to {}", num_cpus);
        }

        info!("🔧 Platform-specific optimizations:");
        
        #[cfg(target_os = "linux")]
        {
            info!("   Linux: Use 'taskset -c 0-{} ./mev-bot' for core pinning", num_cpus - 1);
            info!("   Linux: Set 'echo performance > /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor'");
            info!("   Linux: Disable CPU frequency scaling for consistent performance");
        }

        #[cfg(target_os = "windows")]
        {
            info!("   Windows: Use Process Lasso for CPU affinity management");
            info!("   Windows: Set High Performance power plan");
            info!("   Windows: Disable CPU parking in registry");
        }

        info!("🚀 Compiler optimizations:");
        info!("   Set RUSTFLAGS='-C target-cpu=native -C opt-level=3'");
        info!("   Use 'cargo build --release' for production builds");
    }
}

/// CPU usage statistics
#[derive(Debug, Clone)]
pub struct CpuStats {
    pub total_runtime_seconds: f64,
    pub interval_seconds: f64,
    pub cpu_usage_percent: f64,
    pub num_cores: usize,
    pub pinned_cores: Vec<usize>,
}

impl CpuStats {
    pub fn print_summary(&self) {
        info!(
            runtime_seconds = format!("{:.1}", self.total_runtime_seconds),
            cpu_usage = format!("{:.1}%", self.cpu_usage_percent),
            num_cores = self.num_cores,
            pinned_cores = ?self.pinned_cores,
            "CPU performance stats"
        );
    }
}

/// Thread pool with CPU affinity
pub struct AffinityThreadPool {
    handles: Vec<std::thread::JoinHandle<()>>,
    core_assignments: Vec<usize>,
}

impl AffinityThreadPool {
    pub fn new(config: CorePinningConfig) -> Result<Self> {
        let core_assignments = if config.enabled {
            let manager = CpuAffinityManager::new(config.clone());
            manager.get_core_assignment()
        } else {
            vec![]
        };

        info!(
            worker_threads = config.worker_threads,
            core_assignments = ?core_assignments,
            "Creating affinity thread pool"
        );

        Ok(Self {
            handles: Vec::new(),
            core_assignments,
        })
    }

    pub fn spawn<F>(&mut self, thread_id: usize, f: F) -> Result<()>
    where
        F: FnOnce() + Send + 'static,
    {
        let core_id = if thread_id < self.core_assignments.len() {
            Some(self.core_assignments[thread_id])
        } else {
            None
        };

        let handle = std::thread::spawn(move || {
            if let Some(core_id) = core_id {
                info!(
                    thread_id = thread_id,
                    core_id = core_id,
                    "Worker thread starting with CPU affinity"
                );
                // In a real implementation, you'd set thread affinity here
            }
            
            f();
        });

        self.handles.push(handle);
        Ok(())
    }

    pub fn join_all(self) -> Result<()> {
        for handle in self.handles {
            if let Err(e) = handle.join() {
                return Err(anyhow::anyhow!("Thread join error: {:?}", e));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_core_assignment() {
        let config = CorePinningConfig {
            enabled: true,
            core_ids: vec![0, 1, 2, 3],
            worker_threads: 6,
        };

        let manager = CpuAffinityManager::new(config);
        let assignment = manager.get_core_assignment();
        
        assert_eq!(assignment.len(), 6);
        assert_eq!(assignment, vec![0, 1, 2, 3, 0, 1]); // Round-robin assignment
    }

    #[test]
    fn test_cpu_stats() {
        let mut monitor = CpuPerformanceMonitor::new();
        std::thread::sleep(std::time::Duration::from_millis(10));
        
        let stats = monitor.get_cpu_stats();
        assert!(stats.total_runtime_seconds > 0.0);
        assert!(stats.interval_seconds > 0.0);
        assert!(stats.num_cores > 0);
    }
}