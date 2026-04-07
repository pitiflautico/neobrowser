"""Benchmark infrastructure — logging, stats, resource monitoring."""

import json, time, os, sys, statistics, subprocess, threading
from pathlib import Path
from datetime import datetime

RESULTS_DIR = Path(__file__).parent / 'results'
RESULTS_DIR.mkdir(exist_ok=True)

# ── Resource monitoring ──

def get_process_resources(pid=None):
    """Get CPU% and RAM MB for a process. Uses psutil if available, else ps."""
    pid = pid or os.getpid()
    try:
        import psutil
        p = psutil.Process(pid)
        return {
            'cpu_percent': p.cpu_percent(interval=0.1),
            'ram_mb': p.memory_info().rss / 1024 / 1024,
            'fd_count': p.num_fds() if hasattr(p, 'num_fds') else -1,
        }
    except ImportError:
        pass
    # Fallback: ps command
    try:
        out = subprocess.check_output(['ps', '-o', '%cpu,rss', '-p', str(pid)], text=True)
        lines = out.strip().split('\n')
        if len(lines) >= 2:
            parts = lines[1].split()
            return {
                'cpu_percent': float(parts[0]),
                'ram_mb': int(parts[1]) / 1024,
                'fd_count': -1,
            }
    except Exception:
        pass
    return {'cpu_percent': 0, 'ram_mb': 0, 'fd_count': -1}


class ResourceMonitor:
    """Background thread that samples CPU/RAM at intervals."""
    def __init__(self, pid=None, interval_s=1.0):
        self.pid = pid or os.getpid()
        self.interval = interval_s
        self.samples = []
        self._stop = threading.Event()
        self._thread = None
        self._start_time = None
        self._end_time = None

    def start(self):
        self._stop.clear()
        self._start_time = time.perf_counter()
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()
        return self

    def _run(self):
        while not self._stop.is_set():
            self.samples.append(get_process_resources(self.pid))
            self._stop.wait(self.interval)

    def stop(self):
        self._stop.set()
        self._end_time = time.perf_counter()
        if self._thread:
            self._thread.join(timeout=2)

    def duration_ms(self):
        """Return monitoring duration in ms, or 0 if not stopped yet."""
        if self._start_time is None:
            return 0
        end = self._end_time if self._end_time is not None else time.perf_counter()
        return round((end - self._start_time) * 1000, 1)

    def stats(self):
        if not self.samples:
            return {}
        cpus = [s['cpu_percent'] for s in self.samples]
        rams = [s['ram_mb'] for s in self.samples]
        return {
            'cpu_avg': round(statistics.mean(cpus), 1),
            'cpu_peak': round(max(cpus), 1),
            'ram_avg': round(statistics.mean(rams), 1),
            'ram_peak': round(max(rams), 1),
            'samples': len(self.samples),
        }


# ── Timing ──

class Timer:
    """Context manager for timing."""
    def __init__(self):
        self.start = 0
        self.end = 0
        self.ms = 0

    def __enter__(self):
        self.start = time.perf_counter()
        return self

    def __exit__(self, *args):
        self.end = time.perf_counter()
        self.ms = round((self.end - self.start) * 1000, 1)

    def lap(self):
        return round((time.perf_counter() - self.start) * 1000, 1)


# ── Result logging ──

_results = []

def log_result(test_name, tool, success, duration_ms, **extra):
    """Log a benchmark result."""
    entry = {
        'test_name': test_name,
        'tool': tool,
        'timestamp': datetime.now().isoformat(),
        'success': success,
        'duration_ms': round(duration_ms, 1),
        **extra,
    }
    _results.append(entry)
    status = 'OK' if success else 'FAIL'
    print(f'  [{status}] [{tool}] {test_name}: {duration_ms:.0f}ms', file=sys.stderr)
    return entry


def save_results(filename=None):
    """Save all results to JSON file."""
    if not filename:
        filename = f'bench-{datetime.now().strftime("%Y%m%d-%H%M%S")}.json'
    path = RESULTS_DIR / filename
    path.write_text(json.dumps(_results, indent=2))
    print(f'\nResults saved: {path}', file=sys.stderr)
    return path


def compute_stats(results, key='duration_ms'):
    """Compute p50/p95/p99 from a list of result dicts."""
    vals = sorted(r[key] for r in results if r.get('success'))
    if not vals:
        return {}
    n = len(vals)
    return {
        'count': n,
        'p50': round(vals[int(n * 0.5)], 1),
        'p95': round(vals[int(n * 0.95)], 1) if n >= 5 else round(vals[-1], 1),
        'p99': round(vals[int(n * 0.99)], 1) if n >= 10 else round(vals[-1], 1),
        'mean': round(statistics.mean(vals), 1),
        'min': round(vals[0], 1),
        'max': round(vals[-1], 1),
        'success_rate': round(len(vals) / len(results) * 100, 1),
    }


def print_comparison(test_name, tools_results):
    """Print side-by-side comparison table. Works for 1+ tools."""
    print(f'\n{"="*60}')
    print(f'  {test_name}')
    print(f'{"="*60}')
    header = f'  {"Metric":<20}'
    for tool in tools_results:
        header += f' {tool:>12}'
    print(header)
    print(f'  {"-"*20}' + ''.join(f' {"-"*12}' for _ in tools_results))

    metrics = ['count', 'p50', 'p95', 'p99', 'mean', 'min', 'max', 'success_rate']
    all_stats = {tool: compute_stats(results) for tool, results in tools_results.items()}

    for m in metrics:
        line = f'  {m:<20}'
        for tool in tools_results:
            val = all_stats[tool].get(m, '-')
            if isinstance(val, float):
                suffix = '%' if m == 'success_rate' else 'ms'
                line += f' {val:>10.1f}{suffix}'
            else:
                line += f' {str(val):>12}'
        print(line)
    print()
