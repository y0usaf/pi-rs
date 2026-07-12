import { createPublicKey, verify } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { createRequire } from "node:module";
import { streamGoogleVertex, streamSimpleGoogleVertex } from "../../ref/pi/packages/ai/src/providers/google-vertex.ts";

const spec = JSON.parse(readFileSync(process.argv[2]!, "utf8"));
const fixtureDir = dirname(process.argv[2]!);
const drop = new Set(["host", "content-length", "connection", "accept-encoding", "accept-language", "sec-fetch-mode", "user-agent"]);
const { Gaxios } = createRequire(import.meta.url)(join(process.cwd(), "ref/pi/node_modules/gaxios"));

function normalizedBody(path: string | undefined, headers: Record<string, string>, text: string): any {
	if (!text) return null;
	if (path === "/oauth-token") {
		const form = new URLSearchParams(text);
		const [encodedHeader, encodedPayload, signature] = form.get("assertion")!.split(".");
		const payload = JSON.parse(Buffer.from(encodedPayload!, "base64url").toString());
		payload.exp -= payload.iat;
		payload.iat = 0;
		return {
			grant_type: form.get("grant_type"),
			assertion: {
				header: JSON.parse(Buffer.from(encodedHeader!, "base64url").toString()),
				payload,
				signatureBytes: Buffer.from(signature!, "base64url").length,
				signatureValid: verify(
					"RSA-SHA256",
					Buffer.from(`${encodedHeader}.${encodedPayload}`),
					createPublicKey(readFileSync(join(fixtureDir, "service-account-key.pem"))),
					Buffer.from(signature!, "base64url"),
				),
			},
		};
	}
	return headers["content-type"]?.startsWith("application/json") ? JSON.parse(text) : text;
}

async function run(c: any) {
	const requests: any[] = [];
	let i = 0;
	const server = createServer((req, res) => {
		const bs: Buffer[] = [];
		req.on("data", x => bs.push(x));
		req.on("end", () => {
			const text = Buffer.concat(bs).toString();
			const headers = Object.fromEntries(Object.entries(req.headers)
				.filter(([k]) => !drop.has(k))
				.map(([k, v]) => [k, Array.isArray(v) ? v.join(", ") : v ?? ""])
				.sort(([a], [b]) => a.localeCompare(b)));
			requests.push({ method: req.method, path: req.url, headers, body: normalizedBody(req.url, headers, text) });
			if (req.url === "/token" || req.url === "/oauth-token") {
				res.writeHead(200, { "content-type": "application/json" });
				res.end(JSON.stringify({ access_token: "adc-token", token_type: "Bearer", expires_in: 3600 }));
				return;
			}
			if (req.url === "/subject-text") {
				res.writeHead(200, { "content-type": "text/plain" });
				res.end("url-subject-token");
				return;
			}
			if (req.url === "/subject-json") {
				res.writeHead(200, { "content-type": "application/json" });
				res.end(JSON.stringify({ token: "url-json-token" }));
				return;
			}
			if (req.url === "/sts") {
				res.writeHead(200, { "content-type": "application/json" });
				res.end(JSON.stringify({ access_token: "sts-token", issued_token_type: "urn:ietf:params:oauth:token-type:access_token", token_type: "Bearer", expires_in: 3600 }));
				return;
			}
			if (req.url === "/impersonate") {
				res.writeHead(200, { "content-type": "application/json" });
				res.end(JSON.stringify({ accessToken: "adc-token", expireTime: "2099-01-01T00:00:00Z" }));
				return;
			}
			if (req.url === "/project/123") {
				res.writeHead(200, { "content-type": "application/json" });
				res.end(JSON.stringify({ projectId: "p" }));
				return;
			}
			const r = c.responses[i++] ?? c.responses.at(-1);
			if (!r) { res.destroy(); return; }
			const body = r.chunks ? r.chunks.map((x: any) => `data: ${JSON.stringify(x)}\n\n`).join("") : r.json !== undefined ? JSON.stringify(r.json) : r.text ?? "";
			res.writeHead(r.status, { "content-type": r.chunks ? "text/event-stream" : r.json !== undefined ? "application/json" : "text/plain" });
			res.end(body);
		});
	});
	await new Promise<void>(ok => server.listen(0, "127.0.0.1", ok));
	const url = `http://127.0.0.1:${(server.address() as any).port}`;
	const model = { ...spec.models[c.model], ...(!c.noServerBase ? { baseUrl: url + (c.baseSuffix ?? "") } : {}) };
	let dir: string | undefined;
	const oldCredentials = process.env.GOOGLE_APPLICATION_CREDENTIALS;
	const oldAllowExecutables = process.env.GOOGLE_EXTERNAL_ACCOUNT_ALLOW_EXECUTABLES;
	const originalAdapter = Gaxios.prototype._defaultAdapter;
	if (c.adc) {
		dir = mkdtempSync(join(tmpdir(), "pi-vertex-oracle-"));
		const credentialPath = join(dir, "adc.json");
		let credentials: any;
		if (c.adc === "authorized-user") {
			credentials = { type: "external_account_authorized_user", client_id: "client", client_secret: "secret", refresh_token: "refresh", token_url: `${url}/token` };
		} else if (c.adc === "service-account") {
			credentials = { type: "service_account", project_id: "p", client_email: "test@p.iam.gserviceaccount.com", private_key: readFileSync(join(fixtureDir, "service-account-key.pem"), "utf8") };
			Gaxios.prototype._defaultAdapter = function(config: any) {
				if (config.url.toString() === "https://oauth2.googleapis.com/token") {
					config = { ...config, url: new URL(`${url}/oauth-token`) };
				}
				return originalAdapter.call(this, config);
			};
		} else if (c.adc === "workload-executable" || c.adc === "workload-executable-cached") {
			const executablePath = join(dir, "subject token.mjs");
			const outputFile = join(dir, "executable-output.json");
			writeFileSync(executablePath, `const token = [process.env.GOOGLE_EXTERNAL_ACCOUNT_AUDIENCE, process.env.GOOGLE_EXTERNAL_ACCOUNT_TOKEN_TYPE, process.env.GOOGLE_EXTERNAL_ACCOUNT_INTERACTIVE, process.env.GOOGLE_EXTERNAL_ACCOUNT_OUTPUT_FILE ?? ""].join("|"); process.stdout.write(JSON.stringify({version:1,success:true,token_type:"urn:ietf:params:oauth:token-type:jwt",id_token:token}));`);
			const executable: any = { command: `${process.execPath} "${executablePath}"`, timeout_millis: 5000 };
			if (c.adc === "workload-executable-cached") {
				writeFileSync(outputFile, JSON.stringify({ version: 1, success: true, token_type: "urn:ietf:params:oauth:token-type:jwt", id_token: "cached-executable-token", expiration_time: Math.round(Date.now() / 1000) + 3600 }));
				executable.output_file = outputFile;
				executable.command = "must-not-run";
			}
			process.env.GOOGLE_EXTERNAL_ACCOUNT_ALLOW_EXECUTABLES = "1";
			credentials = {
				type: "external_account",
				audience: "//iam.googleapis.com/projects/123/locations/global/workloadIdentityPools/pool/providers/provider",
				subject_token_type: "urn:ietf:params:oauth:token-type:jwt",
				token_url: `${url}/sts`,
				credential_source: { executable },
				cloud_resource_manager_url: `${url}/project/`,
			};
		} else {
			const usesJson = c.adc === "workload-json-impersonated" || c.adc === "workload-url-json";
			const usesUrl = c.adc === "workload-url-text" || c.adc === "workload-url-json";
			const credentialSource = usesUrl
				? { url: `${url}/subject-${usesJson ? "json" : "text"}`, headers: { "x-subject-header": "present" } }
				: { file: join(dir, "subject-token") };
			if (!usesUrl) {
				writeFileSync(credentialSource.file!, usesJson ? JSON.stringify({ token: "subject-token" }) : "subject-token");
			}
			if (usesJson) {
				Object.assign(credentialSource, { format: { type: "json", subject_token_field_name: "token" } });
			}
			credentials = {
				type: "external_account",
				audience: "//iam.googleapis.com/projects/123/locations/global/workloadIdentityPools/pool/providers/provider",
				subject_token_type: "urn:ietf:params:oauth:token-type:jwt",
				token_url: `${url}/sts`,
				credential_source: credentialSource,
				cloud_resource_manager_url: `${url}/project/`,
				...(c.adc === "workload-json-impersonated" ? { service_account_impersonation_url: `${url}/impersonate`, service_account_impersonation: { token_lifetime_seconds: 1800 } } : {}),
			};
		}

		writeFileSync(credentialPath, JSON.stringify(credentials));
		process.env.GOOGLE_APPLICATION_CREDENTIALS = credentialPath;
	}
	const events: any[] = [];
	let result: any, syncError: string | undefined;
	try {
		const s = c.simple ? streamSimpleGoogleVertex(model, c.context, c.options) : streamGoogleVertex(model, c.context, c.options);
		for await (const e of s) { const { partial, message, error, ...rest } = e as any; events.push(rest); }
		result = { ...await s.result(), timestamp: 0 };
	} catch (e) {
		syncError = e instanceof Error ? e.message : String(e);
	} finally {
		server.close();
		Gaxios.prototype._defaultAdapter = originalAdapter;
		if (oldCredentials === undefined) delete process.env.GOOGLE_APPLICATION_CREDENTIALS;
		else process.env.GOOGLE_APPLICATION_CREDENTIALS = oldCredentials;
		if (oldAllowExecutables === undefined) delete process.env.GOOGLE_EXTERNAL_ACCOUNT_ALLOW_EXECUTABLES;
		else process.env.GOOGLE_EXTERNAL_ACCOUNT_ALLOW_EXECUTABLES = oldAllowExecutables;
		if (dir) rmSync(dir, { recursive: true, force: true });
	}
	return { name: c.name, requests, ...(syncError ? { syncError } : { events, result }) };
}

async function main() {
	const cases = [];
	for (const c of spec.cases) cases.push(await run(c));
	console.log(JSON.stringify({ cases }, null, "\t"));
}
main().catch(error => { console.error(error); process.exitCode = 1; });
