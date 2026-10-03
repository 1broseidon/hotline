export type SettingsSection = "general" | "desks" | "providers" | "tools" | "skills" | "computer" | "secrets" | "remote" | "updates" | "import";

export const SETTINGS_SECTIONS: { id: SettingsSection; title: string }[] = [
	{ id: "general", title: "General" },
	{ id: "desks", title: "Desks" },
	{ id: "providers", title: "Providers" },
	{ id: "tools", title: "Tools" },
	{ id: "skills", title: "Skills" },
	{ id: "computer", title: "Computer" },
	{ id: "secrets", title: "Secrets" },
	{ id: "remote", title: "Remote" },
	{ id: "updates", title: "Updates" },
	{ id: "import", title: "Import" },
];

