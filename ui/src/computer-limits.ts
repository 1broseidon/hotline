/**
 * How much of the desk a teammate's computer may use, as the sliders say
 * it: memory in half gigabytes and CPU in halves, up to what the desk has.
 * The phone's owner card reads the same numbers the same way.
 */

/** What `computer.capacity` answers. */
export type Capacity = {
	runtime: "docker" | "podman" | "container" | null;
	cpus: number;
	memoryBytes: number;
	source: "runtime" | "host" | "default";
};

export const MEMORY_STEP_MB = 512;
export const CPU_STEP = 0.5;
/** A computer with no memory set gets this much. */
export const DEFAULT_MEMORY_MB = 4096;

const MB = 1024 * 1024;

/** The runtime's spelling of a memory limit, in megabytes; anything unreadable is the default, as the desk treats it. */
export function memoryMb(memory: string | undefined): number {
	const match = /^\s*(\d+(?:\.\d+)?)\s*([kmgt]?)b?\s*$/i.exec(memory ?? "");
	if (!match) return DEFAULT_MEMORY_MB;
	const value = Number(match[1]);
	const unit = (match[2] ?? "").toLowerCase();
	const scale: Record<string, number> = { t: 1024 * 1024, g: 1024, m: 1, k: 1 / 1024, "": 1 / MB };
	const mb = value * (scale[unit] ?? 1);
	return mb > 0 ? Math.round(mb) : DEFAULT_MEMORY_MB;
}

/** Whole megabytes, which every runtime reads, Apple's `container` included. */
export function memorySpelling(mb: number): string {
	return `${Math.round(mb)}m`;
}

export function memoryCeilingMb(capacity: Capacity): number {
	return Math.max(MEMORY_STEP_MB, Math.floor(capacity.memoryBytes / MB / MEMORY_STEP_MB) * MEMORY_STEP_MB);
}

export function cpuCeiling(capacity: Capacity): number {
	return Math.max(CPU_STEP, Math.floor(capacity.cpus / CPU_STEP) * CPU_STEP);
}

export function snap(value: number, step: number, min: number, max: number): number {
	return Math.min(max, Math.max(min, Math.round(value / step) * step));
}

export function memoryLabel(mb: number): string {
	if (mb < 1024) return `${Math.round(mb)} MB`;
	const gb = mb / 1024;
	return `${Number.isInteger(gb) ? gb : gb.toFixed(1)} GB`;
}

function cpuCount(cpus: number): string {
	return `${cpus} ${cpus <= 1 ? "CPU" : "CPUs"}`;
}

/** No cap is every CPU the desk has; Apple's runtime gives whole CPUs, so a half there reads as the whole it becomes. */
export function cpuLabel(cpus: number | null | undefined, capacity?: Capacity | null): string {
	if (cpus == null) return capacity ? `All ${cpuCount(capacity.cpus)}` : "No CPU limit";
	return cpuCount(capacity?.runtime === "container" ? Math.max(1, Math.ceil(cpus)) : cpus);
}

/** Where the desk's numbers came from, when it matters. */
export function capacityNote(capacity: Capacity): string | null {
	if (capacity.runtime === null) return "No container runtime found. Install Docker or Podman to run a computer.";
	if (capacity.source === "default") return "Couldn't read this machine's size, so these are safe defaults.";
	if (capacity.runtime === "container") return "Apple container gives whole CPUs.";
	return null;
}
