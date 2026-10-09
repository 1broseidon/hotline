import type { SpeechModel } from "../generated/contract";

/**
 * The desk's own speech models as Settings shows them: what each is, how
 * big, and where its download stands. The desk does the work (voice.md,
 * Hearing on the desk); this only words it.
 */

/** A size the way a download dialog says it: megabytes, decimal, whole. */
export function megabytes(bytes: number): string {
	if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
	return `${Math.max(1, Math.round(bytes / 1e6))} MB`;
}

/** Whether something is still moving, so the window should ask again soon. */
export function busy(models: readonly SpeechModel[]): boolean {
	return models.some((model) => model.state === "downloading" || model.state === "unpacking");
}

/** How far a download has come, 0 to 1. */
export function progress(model: SpeechModel): number {
	if (model.downloadBytes <= 0) return 0;
	return Math.min(1, Math.max(0, (model.receivedBytes ?? 0) / model.downloadBytes));
}

/** A model row's second line. */
export function modelLine(model: SpeechModel): string {
	switch (model.state) {
		case "downloading":
			return `Downloading ${megabytes(model.receivedBytes ?? 0)} of ${megabytes(model.downloadBytes)}`;
		case "unpacking":
			return "Unpacking";
		case "installed":
			return model.diskBytes > 0 ? `${megabytes(model.diskBytes)} on the desk` : "";
		case "available":
			return "";
	}
}

/** Whether the set of installed models differs, which changes what can hear. */
export function installedChanged(before: readonly SpeechModel[], after: readonly SpeechModel[]): boolean {
	const ids = (models: readonly SpeechModel[]) =>
		models
			.filter((model) => model.state === "installed")
			.map((model) => model.id)
			.sort()
			.join("\n");
	return ids(before) !== ids(after);
}
