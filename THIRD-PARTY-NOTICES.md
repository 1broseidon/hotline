# Third-party licenses and notices

Hotline is MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`). What a
release carries from other people falls in four parts: the Rust crates
linked into the binaries, the packages built into the window, the fonts
and prebuilt bundles among those packages, and the speech engine's native
libraries. The speech models are not in a release; each is downloaded when
its owner asks for it.

This is an inventory of what the gates read, written for whoever reviews a
license. It is not a legal opinion. The app bundle does not carry it yet.

## The gate

`make check` runs the dependency gates, `make deps`, whose license half fails on:

- a crate under a license outside the allow list in `deny.toml`
  (`cargo deny --locked check advisories licenses`, every target);
- a window package, anywhere in what `ui/package.json`'s `dependencies`
  reach, under a license outside `REVIEWED` in
  `scripts/window_licenses.py`. Development dependencies are build tools
  and are not walked.

A new license is added to the list with what it obliges, here, in the same
commit. Neither tool reads fonts, a prebuilt bundle's contents or the
speech engine's archive; those are the sections below, re-read when the
package or the pinned archive changes.

## Rust crates

Listed with `cargo deny --locked list`. Nearly every crate is MIT,
Apache-2.0, or a choice of the two, and each keeps its copyright and
license beside a copy. The rest:

| License | Crates | What it obliges |
|---|---|---|
| Apache-2.0 WITH LLVM-exception | rustix, cap-std, wasi, wit-bindgen and their families | As Apache-2.0 |
| BSD-1-, BSD-2-, BSD-3-Clause | fiat-crypto; zerocopy, serial2; brotli, curve25519-dalek, encoding_rs, subtle, zstd, aws-lc-sys, opusic-sys and others | Keep the copyright and license |
| ISC | ring, rustls, rustls-webpki, aws-lc-rs, libloading, untrusted | Keep the copyright and license |
| Zlib | miniz_oxide, zlib-rs, bytemuck, foldhash, the objc2 crates | Do not misrepresent the origin |
| BSL-1.0 | clipboard-win, error-code | Keep the license with source copies |
| Unicode-3.0 | the icu crates, unicode-ident, zerovec, yoke, litemap | Keep the copyright and license |
| bzip2-1.0.6 | libbz2-rs-sys | Keep the copyright and license |
| 0BSD, CC0-1.0, MIT-0, Unlicense | adler2; dunce, constant_time_eq, ppmd-rust; aws-lc-sys; memchr, walkdir, ignore, the grep crates | Nothing |
| CDLA-Permissive-2.0 | webpki-roots, webpki-root-certs (the Mozilla root certificates, as data) | Keep the license text with the data |
| MPL-2.0 | cssparser, selectors, dtoa-short, option-ext, symphonia and its codecs | A changed MPL file is published with its source. Hotline changes none |

r-efi also offers LGPL-2.1-or-later, as an alternative to MIT and
Apache-2.0; the gate passes it on those.

## Window packages

Listed by the gate's own walk, about 230 packages. Most are MIT or ISC; the
rest:

| License | Packages | What it obliges |
|---|---|---|
| Apache-2.0 | chevrotain (Mermaid's parser) | Keep the license and any NOTICE |
| BSD-3-Clause | d3-array, d3-ease, d3-path, d3-sankey, d3-shape, rw | Keep the copyright and license |
| MIT OR Apache-2.0 | `@tauri-apps/api` and the plugins | As either |
| MPL-2.0 OR Apache-2.0 | DOMPurify | Taken as Apache-2.0 |
| Unlicense | robust-predicates | Nothing |
| OFL-1.1 | `@fontsource/ibm-plex-sans`, `@fontsource/ibm-plex-mono` | See Fonts |
| EPL-2.0 | elkjs (Mermaid's ELK layout) | See Bundled builds |
| MIT, undeclared | khroma | Its manifest has no license field; its `license` file is MIT, so the gate is told so by name |

Tailwind is a development dependency, but the stylesheet it generates
carries its preflight, which is MIT.

### Fonts

IBM Plex Sans and IBM Plex Mono, copyright IBM Corp., under the SIL Open
Font License 1.1, from the Fontsource packages. `ui/src/tokens.css` imports
five weights in every script and both formats, so the build emits each as a
WOFF and a WOFF2 file. The OFL lets them be bundled with software and asks
that the copyright and license go with each copy, as separate text or in
the font's own metadata, and that a modified font not use the reserved name.
They are not modified.

### Bundled builds

Mermaid is loaded from its own single-file build (`mermaid.min.js`, see
`ui/src/components/Mermaid.tsx`), which is Mermaid's dependencies compiled
into one script by Mermaid's release, not by this build. Its trailing
"Bundled license information" comment names DOMPurify, lodash-es and
cytoscape. It also carries elkjs, under the Eclipse Public License 2.0,
which that comment does not name: EPL-2.0 lets it ship as object code if
the recipient is told the EPL governs it and where its source is
(<https://github.com/kieler/elkjs>, the version in `ui/bun.lock`).

The rest of the window is compiled by Vite into `ui/dist`, which the app
embeds.

## The speech engine

The desk's local speech-to-text links sherpa-onnx statically from a
prebuilt archive pinned in `scripts/speech-engine.sha256` (v1.13.8). Each
archive holds these libraries:

| Library | License |
|---|---|
| sherpa-onnx | Apache-2.0 |
| ONNX Runtime | MIT, with its own third-party notices |
| kaldi-native-fbank, kaldi-decoder, kaldifst, OpenFst | Apache-2.0 |
| simple-sentencepiece | Apache-2.0 |
| KISS FFT | BSD-3-Clause |
| PortAudio | MIT-style |
| piper-phonemize | MIT |
| eSpeak NG, with its ucd library | GPL-3.0-or-later |

eSpeak NG is the one copyleft library here. sherpa-onnx uses it only for
speech synthesis, which Hotline does not call, and the linker keeps only
what is called: the macOS arm64 debug and release binaries hold no eSpeak NG
symbol (`nm target/release/hotline-app | grep -ci espeak` prints 0), while
piper-phonemize's are there. Linux and Windows builds have not been read the
same way. A sherpa-onnx bump re-reads this table and that count.

The models are downloaded from the sherpa-onnx releases when their owner
installs one, and the window credits each with its license
(`crates/hotline-voice/src/speech/local/install.rs`): Parakeet under
CC BY 4.0, Whisper and Moonshine under MIT.
