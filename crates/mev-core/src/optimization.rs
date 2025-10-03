//! Performance optimization utilities and micro-optimizations for MEV bot
//! 
//! This module contains various performance optimizations including:
//! - Memory pool management and allocation optimization
//! - CPU cache optimization and data structure alignment
//! - Hot path optimizations for critical code paths
//! - Thread pool tuning and work distribution

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::RwLock;
use serde::{Deserialize, Serialize};

/// Memory pool for reusing allocations and reducing GC pressure
pub struct MemoryPool<T> {
    pool: Arc<Mutex<Vec<T>>>,
    factory: Box<dyn Fn() -> T + Send + Sync>,
    max_size: usize,
}

impl<T> MemoryPool<T> {
    pub fn new<F>(factory: F, max_size: usize) -> Self
    where
        F: Fn() -> T + Send + Sync + 'static,
    {
        Self {
            pool: Arc::new(Mutex::new(Vec::with_capacity(max_size))),
            factory: Box::new(factory),
            max_size,
        }
    }
    
    pub fn acquire(&self) -> PooledItem<T> {
        let item = {
            let mut pool = self.pool.lock().unwrap();
            pool.pop().unwrap_or_else(|| (self.factory)())
        };
        
        PooledItem {
            item: Some(item),
            pool: Arc::clone(&self.pool),
        }
    }
    
    pub fn size(&self) -> usize {
        self.pool.lock().unwrap().len()
    }
}

/// RAII wrapper for pooled items that automatically returns to pool on drop
pub struct PooledItem<T> {
    item: Option<T>,
    pool: Arc<Mutex<Vec<T>>>,
}

impl<T> PooledItem<T> {
    pub fn get(&self) -> &T {
        self.item.as_ref().unwrap()
    }
    
    pub fn get_mut(&mut self) -> &mut T {
        self.item.as_mut().unwrap()
    }
}

impl<T> Drop for PooledItem<T> {
    fn drop(&mut self) {
        if let Some(item) = self.item.take() {
            let mut pool = self.pool.lock().unwrap();
            if pool.len() < pool.capacity() {
                pool.push(item);
            }
        }
    }
}

/// Cache-aligned data structures for better CPU cache performance
#[repr(align(64))] // Align to cache line size
pub struct CacheAlignedCounter {
    value: std::sync::atomic::AtomicU64,
    _padding: [u8; 56], // Pad to cache line size
}

impl CacheAlignedCounter {
    pub fn new(initial: u64) -> Self {
        Self {
            value: std::sync::atomic::AtomicU64::new(initial),
            _padding: [0; 56],
        }
    }
    
    pub fn increment(&self) -> u64 {
        self.value.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }
    
    pub fn get(&self) -> u64 {
        self.value.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Hot ABI decoder cache for frequently used function signatures
pub struct HotAbiCache {
    cache: RwLock<HashMap<[u8; 4], CachedAbiDecoder>>,
    hit_counter: CacheAlignedCounter,
    miss_counter: CacheAlignedCounter,
}

#[derive(Clone)]
struct CachedAbiDecoder {
    function_name: String,
    input_types: Vec<String>,
    decoder: Arc<dyn Fn(&[u8]) -> Result<Vec<ethabi::Token>, ethabi::Error> + Send + Sync>,
    last_used: Instant,
    use_count: u64,
}

impl HotAbiCache {
    pub fn new() -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            hit_counter: CacheAlignedCounter::new(0),
            miss_counter: CacheAlignedCounter::new(0),
        }
    }
    
    pub async fn get_decoder(&self, selector: [u8; 4]) -> Option<CachedAbiDecoder> {
        let cache = self.cache.read().await;
        if let Some(decoder) = cache.get(&selector) {
            self.hit_counter.increment();
            Some(decoder.clone())
        } else {
            self.miss_counter.increment();
            None
        }
    }
    
    pub async fn insert_decoder(
        &self,
        selector: [u8; 4],
        function_name: String,
        input_types: Vec<String>,
        decoder: Arc<dyn Fn(&[u8]) -> Result<Vec<ethabi::Token>, ethabi::Error> + Send + Sync>,
    ) {
        let cached_decoder = CachedAbiDecoder {
            function_name,
            input_types,
            decoder,
            last_used: Instant::now(),
            use_count: 1,
        };
        
        let mut cache = self.cache.write().await;
        cache.insert(selector, cached_decoder);
        
        // Evict old entries if cache is too large
        if cache.len() > 1000 {
            self.evict_lru(&mut cache).await;
        }
    }
    
    async fn evict_lru(&self, cache: &mut HashMap<[u8; 4], CachedAbiDecoder>) {
        let mut entries: Vec<_> = cache.iter().collect();
        entries.sort_by_key(|(_, decoder)| decoder.last_used);
        
        // Remove oldest 10% of entries
        let remove_count = cache.len() / 10;
        for (selector, _) in entries.iter().take(remove_count) {
            cache.remove(selector);
        }
    }
    
    pub fn get_stats(&self) -> CacheStats {
        let hits = self.hit_counter.get();
        let misses = self.miss_counter.get();
        let total = hits + misses;
        
        CacheStats {
            hits,
            misses,
            hit_rate: if total > 0 { hits as f64 / total as f64 } else { 0.0 },
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub hit_rate: f64,
}

/// Optimized thread pool for CPU-intensive tasks
pub struct OptimizedThreadPool {
    pool: rayon::ThreadPool,
    task_counter: CacheAlignedCounter,
    completion_counter: CacheAlignedCounter,
}

impl OptimizedThreadPool {
    pub fn new(num_threads: usize) -> Result<Self, rayon::ThreadPoolBuildError> {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(num_threads)
            .thread_name(|i| format!("mev-worker-{}", i))
            .build()?;
        
        Ok(Self {
            pool,
            task_counter: CacheAlignedCounter::new(0),
            completion_counter: CacheAlignedCounter::new(0),
        })
    }
    
    pub fn execute<F>(&self, task: F)
    where
        F: FnOnce() + Send + 'static,
    {
        self.task_counter.increment();
        let completion_counter = &self.completion_counter;
        
        self.pool.spawn(move || {
            task();
            completion_counter.increment();
        });
    }
    
    pub fn get_stats(&self) -> ThreadPoolStats {
        let submitted = self.task_counter.get();
        let completed = self.completion_counter.get();
        
        ThreadPoolStats {
            submitted_tasks: submitted,
            completed_tasks: completed,
            pending_tasks: submitted.saturating_sub(completed),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ThreadPoolStats {
    pub submitted_tasks: u64,
    pub completed_tasks: u64,
    pub pending_tasks: u64,
}

/// Memory-efficient ring buffer for high-throughput data processing
pub struct OptimizedRingBuffer<T> {
    buffer: Vec<Option<T>>,
    head: std::sync::atomic::AtomicUsize,
    tail: std::sync::atomic::AtomicUsize,
    capacity: usize,
}

impl<T> OptimizedRingBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        let mut buffer = Vec::with_capacity(capacity);
        buffer.resize_with(capacity, || None);
        
        Self {
            buffer,
            head: std::sync::atomic::AtomicUsize::new(0),
            tail: std::sync::atomic::AtomicUsize::new(0),
            capacity,
        }
    }
    
    pub fn try_push(&self, item: T) -> Result<(), T> {
        let current_tail = self.tail.load(std::sync::atomic::Ordering::Relaxed);
        let next_tail = (current_tail + 1) % self.capacity;
        let current_head = self.head.load(std::sync::atomic::Ordering::Acquire);
        
        if next_tail == current_head {
            return Err(item); // Buffer full
        }
        
        // Safety: We've checked that the slot is available
        unsafe {
            let slot = self.buffer.as_ptr().add(current_tail) as *mut Option<T>;
            std::ptr::write(slot, Some(item));
        }
        
        self.tail.store(next_tail, std::sync::atomic::Ordering::Release);
        Ok(())
    }
    
    pub fn try_pop(&self) -> Option<T> {
        let current_head = self.head.load(std::sync::atomic::Ordering::Relaxed);
        let current_tail = self.tail.load(std::sync::atomic::Ordering::Acquire);
        
        if current_head == current_tail {
            return None; // Buffer empty
        }
        
        // Safety: We've checked that there's an item available
        let item = unsafe {
            let slot = self.buffer.as_ptr().add(current_head) as *mut Option<T>;
            std::ptr::replace(slot, None)
        };
        
        let next_head = (current_head + 1) % self.capacity;
        self.head.store(next_head, std::sync::atomic::Ordering::Release);
        
        item
    }
    
    pub fn len(&self) -> usize {
        let head = self.head.load(std::sync::atomic::Ordering::Relaxed);
        let tail = self.tail.load(std::sync::atomic::Ordering::Relaxed);
        
        if tail >= head {
            tail - head
        } else {
            self.capacity - head + tail
        }
    }
    
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    
    pub fn is_full(&self) -> bool {
        self.len() == self.capacity - 1
    }
}

/// CPU affinity and core pinning utilities
pub struct CpuOptimizer;

impl CpuOptimizer {
    /// Pin current thread to specific CPU cores
    #[cfg(target_os = "linux")]
    pub fn pin_to_cores(cores: &[usize]) -> Result<(), Box<dyn std::error::Error>> {
        use libc::{cpu_set_t, sched_setaffinity, CPU_SET, CPU_ZERO};
        
        let mut cpu_set: cpu_set_t = unsafe { std::mem::zeroed() };
        unsafe { CPU_ZERO(&mut cpu_set) };
        
        for &core in cores {
            unsafe { CPU_SET(core, &mut cpu_set) };
        }
        
        let result = unsafe {
            sched_setaffinity(0, std::mem::size_of::<cpu_set_t>(), &cpu_set)
        };
        
        if result != 0 {
            return Err("Failed to set CPU affinity".into());
        }
        
        Ok(())
    }
    
    #[cfg(not(target_os = "linux"))]
    pub fn pin_to_cores(_cores: &[usize]) -> Result<(), Box<dyn std::error::Error>> {
        // CPU pinning not supported on this platform
        Ok(())
    }
    
    /// Get number of available CPU cores
    pub fn get_cpu_count() -> usize {
        num_cpus::get()
    }
    
    /// Get optimal thread count for CPU-bound tasks
    pub fn get_optimal_thread_count() -> usize {
        let cpu_count = Self::get_cpu_count();
        // Leave one core for system tasks
        (cpu_count - 1).max(1)
    }
}

/// Performance monitoring and profiling utilities
pub struct PerformanceMonitor {
    metrics: Arc<Mutex<HashMap<String, PerformanceMetric>>>,
}

#[derive(Debug, Clone)]
struct PerformanceMetric {
    total_time: Duration,
    call_count: u64,
    min_time: Duration,
    max_time: Duration,
}

impl PerformanceMonitor {
    pub fn new() -> Self {
        Self {
            metrics: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    
    pub fn time_operation<F, R>(&self, name: &str, operation: F) -> R
    where
        F: FnOnce() -> R,
    {
        let start = Instant::now();
        let result = operation();
        let duration = start.elapsed();
        
        self.record_timing(name, duration);
        result
    }
    
    pub fn record_timing(&self, name: &str, duration: Duration) {
        let mut metrics = self.metrics.lock().unwrap();
        let metric = metrics.entry(name.to_string()).or_insert(PerformanceMetric {
            total_time: Duration::from_nanos(0),
            call_count: 0,
            min_time: Duration::from_secs(u64::MAX),
            max_time: Duration::from_nanos(0),
        });
        
        metric.total_time += duration;
        metric.call_count += 1;
        metric.min_time = metric.min_time.min(duration);
        metric.max_time = metric.max_time.max(duration);
    }
    
    pub fn get_stats(&self) -> HashMap<String, PerformanceStats> {
        let metrics = self.metrics.lock().unwrap();
        metrics
            .iter()
            .map(|(name, metric)| {
                let avg_time = if metric.call_count > 0 {
                    metric.total_time / metric.call_count as u32
                } else {
                    Duration::from_nanos(0)
                };
                
                let stats = PerformanceStats {
                    call_count: metric.call_count,
                    total_time_ms: metric.total_time.as_millis() as u64,
                    avg_time_ms: avg_time.as_millis() as u64,
                    min_time_ms: metric.min_time.as_millis() as u64,
                    max_time_ms: metric.max_time.as_millis() as u64,
                };
                
                (name.clone(), stats)
            })
            .collect()
    }
    
    pub fn reset(&self) {
        let mut metrics = self.metrics.lock().unwrap();
        metrics.clear();
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PerformanceStats {
    pub call_count: u64,
    pub total_time_ms: u64,
    pub avg_time_ms: u64,
    pub min_time_ms: u64,
    pub max_time_ms: u64,
}

/// Macro for easy performance timing
#[macro_export]
macro_rules! time_operation {
    ($monitor:expr, $name:expr, $operation:expr) => {
        $monitor.time_operation($name, || $operation)
    };
}

/// Optimized hash map with pre-allocated capacity and custom hasher
pub type FastHashMap<K, V> = HashMap<K, V, ahash::RandomState>;

/// Create a fast hash map with pre-allocated capacity
pub fn create_fast_hashmap<K, V>(capacity: usize) -> FastHashMap<K, V> {
    HashMap::with_capacity_and_hasher(capacity, ahash::RandomState::new())
}

/// Memory allocation tracking for debugging memory usage
pub struct MemoryTracker {
    allocations: Arc<Mutex<HashMap<String, AllocationInfo>>>,
}

#[derive(Debug, Clone)]
struct AllocationInfo {
    size: usize,
    count: usize,
    peak_size: usize,
    peak_count: usize,
}

impl MemoryTracker {
    pub fn new() -> Self {
        Self {
            allocations: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    
    pub fn track_allocation(&self, category: &str, size: usize) {
        let mut allocations = self.allocations.lock().unwrap();
        let info = allocations.entry(category.to_string()).or_insert(AllocationInfo {
            size: 0,
            count: 0,
            peak_size: 0,
            peak_count: 0,
        });
        
        info.size += size;
        info.count += 1;
        info.peak_size = info.peak_size.max(info.size);
        info.peak_count = info.peak_count.max(info.count);
    }
    
    pub fn track_deallocation(&self, category: &str, size: usize) {
        let mut allocations = self.allocations.lock().unwrap();
        if let Some(info) = allocations.get_mut(category) {
            info.size = info.size.saturating_sub(size);
            info.count = info.count.saturating_sub(1);
        }
    }
    
    pub fn get_memory_stats(&self) -> HashMap<String, MemoryStats> {
        let allocations = self.allocations.lock().unwrap();
        allocations
            .iter()
            .map(|(category, info)| {
                let stats = MemoryStats {
                    current_size: info.size,
                    current_count: info.count,
                    peak_size: info.peak_size,
                    peak_count: info.peak_count,
                };
                (category.clone(), stats)
            })
            .collect()
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MemoryStats {
    pub current_size: usize,
    pub current_count: usize,
    pub peak_size: usize,
    pub peak_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_memory_pool() {
        let pool = MemoryPool::new(|| Vec::<u8>::new(), 10);
        
        let mut item1 = pool.acquire();
        item1.get_mut().push(42);
        assert_eq!(item1.get()[0], 42);
        
        drop(item1);
        assert_eq!(pool.size(), 1);
        
        let item2 = pool.acquire();
        assert_eq!(item2.get().len(), 1); // Reused the previous vector
    }
    
    #[test]
    fn test_cache_aligned_counter() {
        let counter = CacheAlignedCounter::new(0);
        assert_eq!(counter.get(), 0);
        
        counter.increment();
        assert_eq!(counter.get(), 1);
        
        counter.increment();
        assert_eq!(counter.get(), 2);
    }
    
    #[test]
    fn test_optimized_ring_buffer() {
        let buffer = OptimizedRingBuffer::new(4);
        
        assert!(buffer.is_empty());
        assert!(!buffer.is_full());
        
        // Fill buffer
        assert!(buffer.try_push(1).is_ok());
        assert!(buffer.try_push(2).is_ok());
        assert!(buffer.try_push(3).is_ok());
        
        assert!(buffer.is_full());
        assert!(buffer.try_push(4).is_err()); // Should fail when full
        
        // Empty buffer
        assert_eq!(buffer.try_pop(), Some(1));
        assert_eq!(buffer.try_pop(), Some(2));
        assert_eq!(buffer.try_pop(), Some(3));
        assert_eq!(buffer.try_pop(), None); // Should be empty
        
        assert!(buffer.is_empty());
    }
    
    #[test]
    fn test_performance_monitor() {
        let monitor = PerformanceMonitor::new();
        
        let result = monitor.time_operation("test_op", || {
            std::thread::sleep(Duration::from_millis(10));
            42
        });
        
        assert_eq!(result, 42);
        
        let stats = monitor.get_stats();
        assert!(stats.contains_key("test_op"));
        
        let test_stats = &stats["test_op"];
        assert_eq!(test_stats.call_count, 1);
        assert!(test_stats.avg_time_ms >= 10);
    }
    
    #[test]
    fn test_memory_tracker() {
        let tracker = MemoryTracker::new();
        
        tracker.track_allocation("test_category", 1024);
        tracker.track_allocation("test_category", 512);
        
        let stats = tracker.get_memory_stats();
        let test_stats = &stats["test_category"];
        
        assert_eq!(test_stats.current_size, 1536);
        assert_eq!(test_stats.current_count, 2);
        assert_eq!(test_stats.peak_size, 1536);
        assert_eq!(test_stats.peak_count, 2);
        
        tracker.track_deallocation("test_category", 512);
        
        let stats = tracker.get_memory_stats();
        let test_stats = &stats["test_category"];
        
        assert_eq!(test_stats.current_size, 1024);
        assert_eq!(test_stats.current_count, 1);
        assert_eq!(test_stats.peak_size, 1536); // Peak should remain
        assert_eq!(test_stats.peak_count, 2);
    }
}