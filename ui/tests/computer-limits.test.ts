import { describe, expect, test } from "bun:test";
import { type Capacity, capacityNote, cpuCeiling, cpuLabel, memoryCeilingMb, memoryLabel, memoryMb, memorySpelling, snap } from "../src/computer-limits";

const GiB = 1024 * 1024 * 1024;
const docker: Capacity = { runtime: "docker", cpus: 7, memoryBytes: 15.6 * GiB, source: "runtime" };

describe("Computer limits", () => {
	test("memory reads every spelling the runtimes take and writes whole megabytes", () => {
		expect(memoryMb("4g")).toBe(4096);
		expect(memoryMb("4.5G")).toBe(4608);
		expect(memoryMb("512m")).toBe(512);
		expect(memoryMb(undefined)).toBe(4096);
		expect(memoryMb("lots")).toBe(4096);
		expect(memorySpelling(4608)).toBe("4608m");
	});

	test("the sliders stop at what the desk has, on half steps", () => {
		expect(memoryCeilingMb(docker)).toBe(15.5 * 1024);
		expect(cpuCeiling(docker)).toBe(7);
		expect(cpuCeiling({ ...docker, cpus: 3.8 })).toBe(3.5);
		expect(snap(2.26, 0.5, 0.5, 7)).toBe(2.5);
		expect(snap(9, 0.5, 0.5, 7)).toBe(7);
	});

	test("limits read as a person says them", () => {
		expect(memoryLabel(4608)).toBe("4.5 GB");
		expect(memoryLabel(512)).toBe("512 MB");
		expect(cpuLabel(0.5)).toBe("0.5 CPU");
		expect(cpuLabel(null)).toBe("No CPU limit");
		expect(cpuLabel(2)).toBe("2 CPUs");
		expect(cpuLabel(null, docker)).toBe("All 7 CPUs");
		expect(cpuLabel(1.5, { ...docker, runtime: "container" })).toBe("2 CPUs");
	});

	test("the desk says why its numbers may not be the whole story", () => {
		expect(capacityNote(docker)).toBeNull();
		expect(capacityNote({ ...docker, runtime: null })).toContain("Docker or Podman");
		expect(capacityNote({ ...docker, source: "default" })).toContain("safe defaults");
	});
});
