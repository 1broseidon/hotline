import { type ReactNode, useEffect, useState } from "react";
import { createPortal } from "react-dom";

/**
 * A picture over the whole window: a diagram, a picture a teammate sent, a
 * capture of a computer's screen. It opens fitted to the window; pressing the
 * picture shows it at its own size, scrolling when that is bigger. Escape, the
 * close button or a press outside the picture closes it.
 *
 * The window's own, rather than the system's viewer: handing a file to
 * whatever the desktop has registered for it fails on enough machines that a
 * picture in a chat should never depend on it.
 */
export function Viewer({ src, alt, onClose, actions }: { src: string; alt: string; onClose(): void; actions?: ReactNode }) {
	const [actual, setActual] = useState(false);
	useEffect(() => {
		const key = (event: KeyboardEvent) => {
			if (event.key !== "Escape") return;
			event.stopPropagation();
			onClose();
		};
		window.addEventListener("keydown", key, true);
		return () => window.removeEventListener("keydown", key, true);
	}, [onClose]);
	return createPortal(
		<div
			className="viewer"
			role="dialog"
			aria-modal="true"
			aria-label={alt}
			onClick={(event) => {
				if (event.target === event.currentTarget) onClose();
			}}
		>
			<div className="viewer-bar">
				{actions}
				<button type="button" className="control btn btn-sm" onClick={onClose}>
					Close
				</button>
			</div>
			<div
				className={`viewer-stage ${actual ? "viewer-actual" : ""}`}
				onClick={(event) => {
					if (event.target === event.currentTarget) onClose();
				}}
			>
				<img
					src={src}
					alt={alt}
					draggable={false}
					title={actual ? "Fit to the window" : "Show at full size"}
					onClick={() => setActual((was) => !was)}
				/>
			</div>
		</div>,
		document.body,
	);
}
