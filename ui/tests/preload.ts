import { afterEach, mock } from "bun:test";

/**
 * After every test, in every file, every spy is put back. The files share
 * one module registry, so a spy on a shared object (`wire.command`) that a
 * test left in place, or put back only after the next file had begun,
 * answered that next file's calls: on Ubuntu CI the relay picker and the
 * teammate's modes read a mock's `{}` as the desk's. No test sets a spy up
 * once for a whole file, so none loses one it needs. Module mocks
 * (`mock.module`) are untouched.
 */
afterEach(() => {
	mock.restore();
});
