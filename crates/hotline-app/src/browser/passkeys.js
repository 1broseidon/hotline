// Passkeys in a teammate's desk browser (passkeys.rs). Runs in the page's
// own world, in every frame, before the page's scripts: a WebAuthn call
// goes to the desk, which answers with the teammate's own passkey from the
// vault, never Apple's. It carries the call's options and nothing else; the
// desk takes the caller's origin from WebKit, so whatever a page writes
// here about where it is changes nothing. No private key ever comes back:
// only the signature, or the new passkey's public key.
(() => {
	if (!globalThis.isSecureContext) return;
	const handler = () => globalThis.webkit?.messageHandlers?.hotlinePasskeys;
	const refused = (name, message) => new DOMException(message, name);

	const bytes = (data) =>
		data instanceof ArrayBuffer ? new Uint8Array(data) : new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
	const toBase64url = (data) => {
		let text = "";
		for (const byte of bytes(data)) text += String.fromCharCode(byte);
		return btoa(text).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
	};
	const fromBase64url = (text) => {
		const padded = text.replace(/-/g, "+").replace(/_/g, "/") + "===".slice((text.length + 3) % 4);
		const plain = atob(padded);
		const out = new Uint8Array(plain.length);
		for (let i = 0; i < plain.length; i += 1) out[i] = plain.charCodeAt(i);
		return out.buffer;
	};
	const descriptors = (list) =>
		(list ?? []).map((credential) => ({ type: credential.type, id: toBase64url(credential.id), transports: credential.transports }));

	const requestJSON = (options) => ({
		challenge: toBase64url(options.challenge),
		rpId: options.rpId,
		allowCredentials: descriptors(options.allowCredentials),
		userVerification: options.userVerification,
	});
	const creationJSON = (options) => ({
		challenge: toBase64url(options.challenge),
		rp: { id: options.rp?.id, name: options.rp?.name },
		user: { id: toBase64url(options.user.id), name: options.user.name, displayName: options.user.displayName },
		pubKeyCredParams: options.pubKeyCredParams,
		excludeCredentials: descriptors(options.excludeCredentials),
	});

	// The answer as the objects a page expects, on the platform's own
	// prototypes where it has them, so `instanceof` holds.
	const shaped = (prototype, fields) => {
		const made = Object.create(prototype ?? Object.prototype);
		for (const [key, value] of Object.entries(fields)) {
			Object.defineProperty(made, key, { value, enumerable: typeof value !== "function" });
		}
		return made;
	};
	const credential = (json) => {
		const answer = json.response;
		const response =
			answer.attestationObject !== undefined
				? shaped(globalThis.AuthenticatorAttestationResponse?.prototype, {
						clientDataJSON: fromBase64url(answer.clientDataJSON),
						attestationObject: fromBase64url(answer.attestationObject),
						getAuthenticatorData: () => fromBase64url(answer.authenticatorData),
						getPublicKey: () => fromBase64url(answer.publicKey),
						getPublicKeyAlgorithm: () => answer.publicKeyAlgorithm,
						getTransports: () => [...answer.transports],
					})
				: shaped(globalThis.AuthenticatorAssertionResponse?.prototype, {
						clientDataJSON: fromBase64url(answer.clientDataJSON),
						authenticatorData: fromBase64url(answer.authenticatorData),
						signature: fromBase64url(answer.signature),
						userHandle: answer.userHandle == null ? null : fromBase64url(answer.userHandle),
					});
		return shaped(globalThis.PublicKeyCredential?.prototype, {
			id: json.id,
			rawId: fromBase64url(json.rawId),
			type: json.type,
			authenticatorAttachment: json.authenticatorAttachment,
			response,
			getClientExtensionResults: () => json.clientExtensionResults,
			toJSON: () => json,
		});
	};

	const aborted = (signal) => signal.reason ?? refused("AbortError", "The operation was aborted.");
	const ask = (kind, encode, options, signal) =>
		new Promise((resolve, reject) => {
			if (signal?.aborted) return reject(aborted(signal));
			let encoded;
			try {
				encoded = encode(options);
			} catch {
				return reject(new TypeError("The passkey request is missing a field it needs."));
			}
			const desk = handler();
			if (!desk) return reject(refused("NotAllowedError", "Passkeys are not available in this browser yet."));
			signal?.addEventListener("abort", () => reject(aborted(signal)), { once: true });
			desk.postMessage(JSON.stringify({ kind, options: encoded })).then(
				(text) => {
					const answer = JSON.parse(text);
					if (answer.error) reject(refused(answer.error.name, answer.error.message));
					else resolve(credential(answer.credential));
				},
				() => reject(refused("NotAllowedError", "The desk did not answer the passkey request.")),
			);
		});

	// A conditional request waits for an autofill pick this browser never
	// offers: it stays pending until the page gives up on it.
	const never = (signal) => new Promise((_, reject) => signal?.addEventListener("abort", () => reject(aborted(signal)), { once: true }));

	const container = globalThis.CredentialsContainer?.prototype;
	const originalGet = container?.get;
	const originalCreate = container?.create;
	const get = function (options) {
		if (!options?.publicKey) {
			return originalGet ? originalGet.call(this, options) : Promise.reject(refused("NotSupportedError", "Only passkeys are supported."));
		}
		if (options.mediation === "conditional") return never(options.signal);
		return ask("get", requestJSON, options.publicKey, options.signal);
	};
	const create = function (options) {
		if (!options?.publicKey) {
			return originalCreate ? originalCreate.call(this, options) : Promise.reject(refused("NotSupportedError", "Only passkeys are supported."));
		}
		return ask("create", creationJSON, options.publicKey, options.signal);
	};
	if (container) {
		Object.defineProperty(container, "get", { value: get, writable: true, configurable: true });
		Object.defineProperty(container, "create", { value: create, writable: true, configurable: true });
	} else {
		const credentials = { get, create };
		Object.defineProperty(Navigator.prototype, "credentials", { get: () => credentials, configurable: true });
	}

	let PublicKeyCredential = globalThis.PublicKeyCredential;
	if (!PublicKeyCredential) {
		PublicKeyCredential = function PublicKeyCredential() {
			throw new TypeError("Illegal constructor");
		};
		Object.defineProperty(globalThis, "PublicKeyCredential", { value: PublicKeyCredential, writable: true, configurable: true });
	}
	const statics = {
		isUserVerifyingPlatformAuthenticatorAvailable: async () => true,
		isConditionalMediationAvailable: async () => false,
		getClientCapabilities: async () => ({
			conditionalCreate: false,
			conditionalGet: false,
			hybridTransport: false,
			passkeyPlatformAuthenticator: true,
			userVerifyingPlatformAuthenticator: true,
		}),
	};
	for (const [name, value] of Object.entries(statics)) {
		Object.defineProperty(PublicKeyCredential, name, { value, writable: true, configurable: true });
	}
})();
