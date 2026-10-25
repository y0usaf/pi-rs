function uses() {
	return (globalThis.__piExtensionUses ??= new Set());
}

export function use(name) {
	uses().add(name);
}

export function drain() {
	const used = [...uses()];
	uses().clear();
	return used;
}

export function stub(name) {
	return () => {
		use(name);
	};
}

export function component(name) {
	return class {
		constructor() {
			use(name);
		}
	};
}
