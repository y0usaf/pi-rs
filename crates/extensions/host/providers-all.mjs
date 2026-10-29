import * as upstream from "./all-upstream.js";

export * from "./all-upstream.js";

const logins = new Map();

/** Upstream's providers, each built-in login keeping one set of functions, so a login an extension copies still names the flow it runs. */
export function builtinProviders() {
	return upstream.builtinProviders().map((provider) => {
		const oauth = provider.auth?.oauth;
		if (!oauth) return provider;
		if (!logins.has(provider.id)) logins.set(provider.id, { login: oauth.login, refresh: oauth.refresh, toAuth: oauth.toAuth });
		return { ...provider, auth: { ...provider.auth, oauth: { ...oauth, ...logins.get(provider.id) } } };
	});
}

/** The built-in provider whose login `oauth` runs, when it reuses one. */
export function flowOf(oauth) {
	if (logins.size === 0) builtinProviders();
	for (const [id, login] of logins) if (login.login === oauth?.login) return id;
	return undefined;
}
