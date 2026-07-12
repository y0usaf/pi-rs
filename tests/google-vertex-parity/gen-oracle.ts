import { createHash, createHmac, createPublicKey, verify } from "node:crypto";
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

function awsHmac(key: Buffer | string, text: string): Buffer {
	return createHmac("sha256", key).update(text).digest();
}

function normalizeAwsSubject(encoded: string): any {
	const value = JSON.parse(decodeURIComponent(encoded));
	const headers = Object.fromEntries(value.headers.map(({ key, value }: any) => [key, value]));
	const authorization = headers.authorization as string;
	const match = authorization.match(/^AWS4-HMAC-SHA256 Credential=([^/]+)\/(\d{8})\/([^/]+)\/([^/]+)\/aws4_request, SignedHeaders=([^,]+), Signature=([0-9a-f]+)$/);
	if (!match) throw new Error(`invalid AWS authorization: ${authorization}`);
	const [, accessKey, date, region, service, signedHeaders, signature] = match;
	const secret = accessKey === "env-access" ? "env-secret" : "metadata-secret";
	const signed = signedHeaders!.split(";");
	const canonicalHeaders = signed.map(name => `${name}:${headers[name]}\n`).join("");
	const parsed = new URL(value.url);
	const emptyHash = createHash("sha256").update("").digest("hex");
	const canonical = `${value.method}\n${parsed.pathname}\n${parsed.search.slice(1)}\n${canonicalHeaders}\n${signedHeaders}\n${emptyHash}`;
	const stringToSign = `AWS4-HMAC-SHA256\n${headers["x-amz-date"]}\n${date}/${region}/${service}/aws4_request\n${createHash("sha256").update(canonical).digest("hex")}`;
	const kDate = awsHmac(`AWS4${secret}`, date);
	const kRegion = awsHmac(kDate, region);
	const kService = awsHmac(kRegion, service);
	const kSigning = awsHmac(kService, "aws4_request");
	const expected = createHmac("sha256", kSigning).update(stringToSign).digest("hex");
	value.url = `{AWS_ORIGIN}${parsed.pathname}${parsed.search}`;
	value.headers = value.headers.map((header: any) => {
		if (header.key === "host") return { ...header, value: "{AWS_HOST}" };
		if (header.key === "x-amz-date") return { ...header, value: "{AWS_DATE}" };
		if (header.key === "authorization") return { key: header.key, value: { accessKey, date: "{AWS_DATE}", region, service, signedHeaders, signatureValid: signature === expected } };
		return header;
	});
	return value;
}

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
	if (path === "/sts") {
		const form = new URLSearchParams(text);
		const entries = Object.fromEntries(form);
		if (entries.subject_token_type === "urn:ietf:params:aws:token-type:aws4_request") entries.subject_token = normalizeAwsSubject(entries.subject_token);
		return entries;
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
			if (req.url === "/aws-imds-token") {
				res.writeHead(200, { "content-type": "text/plain" });
				res.end("imds-token");
				return;
			}
			if (req.url === "/aws-region") {
				res.writeHead(200, { "content-type": "text/plain" });
				res.end("us-east-2b");
				return;
			}
			if (req.url === "/aws-creds") {
				res.writeHead(200, { "content-type": "text/plain" });
				res.end("fixture-role");
				return;
			}
			if (req.url === "/aws-creds/fixture-role") {
				res.writeHead(200, { "content-type": "application/json" });
				res.end(JSON.stringify({ AccessKeyId: "metadata-access", SecretAccessKey: "metadata-secret", Token: "metadata-session" }));
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
	const oldAws = Object.fromEntries(["AWS_REGION", "AWS_DEFAULT_REGION", "AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY", "AWS_SESSION_TOKEN"].map(name => [name, process.env[name]]));
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
		} else if (c.adc === "workload-aws-env" || c.adc === "workload-aws-metadata") {
			const source: any = {
				environment_id: "aws1",
				regional_cred_verification_url: `${url}/aws-verify?Action=GetCallerIdentity&Version=2011-06-15`,
			};
			if (c.adc === "workload-aws-env") {
				process.env.AWS_REGION = "us-west-1";
				process.env.AWS_ACCESS_KEY_ID = "env-access";
				process.env.AWS_SECRET_ACCESS_KEY = "env-secret";
				process.env.AWS_SESSION_TOKEN = "env-session";
			} else {
				for (const name of Object.keys(oldAws)) delete process.env[name];
				Object.assign(source, { region_url: `${url}/aws-region`, url: `${url}/aws-creds`, imdsv2_session_token_url: `${url}/aws-imds-token` });
			}
			credentials = {
				type: "external_account",
				audience: "//iam.googleapis.com/projects/123/locations/global/workloadIdentityPools/pool/providers/provider",
				subject_token_type: "urn:ietf:params:aws:token-type:aws4_request",
				token_url: `${url}/sts`,
				credential_source: source,
				cloud_resource_manager_url: `${url}/project/`,
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
		for (const [name, value] of Object.entries(oldAws)) {
			if (value === undefined) delete process.env[name];
			else process.env[name] = value;
		}
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
