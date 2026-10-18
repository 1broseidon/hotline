import { Window } from "happy-dom";

/**
 * A fresh happy-dom window, installed as the globals React and the window's
 * modules read, for one test or one test file's beforeEach. `extra` adds
 * globals a test needs beyond the document and its storage, such as a fake
 * `WebSocket`. React probes the DOM when it loads, so import it after this.
 * `teardown` puts back whatever the globals were before, then closes the
 * window.
 */
export function installDom(extra: Record<string, unknown> = {}): { dom: Window; teardown(): Promise<void> } {
	const dom = new Window();
	const globals: Record<string, unknown> = {
		window: dom,
		document: dom.document,
		navigator: dom.navigator,
		localStorage: dom.localStorage,
		IS_REACT_ACT_ENVIRONMENT: true,
		...extra,
	};
	const originals = new Map<string, PropertyDescriptor | undefined>();
	for (const [key, value] of Object.entries(globals)) {
		originals.set(key, Object.getOwnPropertyDescriptor(globalThis, key));
		Object.defineProperty(globalThis, key, { value, writable: true, configurable: true });
	}
	return {
		dom,
		async teardown() {
			for (const [key, descriptor] of originals) {
				if (descriptor) Object.defineProperty(globalThis, key, descriptor);
				else Reflect.deleteProperty(globalThis, key);
			}
			await dom.happyDOM.close();
		},
	};
}
