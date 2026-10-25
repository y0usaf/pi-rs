const hostModules = new URL("./node_modules/", import.meta.url);

const missing = new Set(["ERR_MODULE_NOT_FOUND", "MODULE_NOT_FOUND", "ERR_UNSUPPORTED_DIR_IMPORT"]);

const isBare = (specifier) => !specifier.startsWith(".") && !specifier.startsWith("/") && !specifier.includes(":");

export async function resolve(specifier, context, next) {
	try {
		return await next(specifier, context);
	} catch (error) {
		if (!missing.has(error?.code)) throw error;
		if (isBare(specifier)) return next(specifier, { ...context, parentURL: hostModules.href });
		if (specifier.endsWith(".js")) return next(`${specifier.slice(0, -3)}.ts`, context);
		throw error;
	}
}
