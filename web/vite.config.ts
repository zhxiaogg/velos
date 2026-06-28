import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// The apiserver. All browser traffic is proxied here through `/velos/*` so the
// browser never sees a credential and there are no CORS concerns.
const VELOS = process.env.VELOS_SERVER ?? "http://127.0.0.1:8080";

// Module-level credential shared between the auth plugin (which mints it) and
// the proxy (which injects it as a Bearer header on every forwarded request).
let credential = "";

async function ensureCredential(log: (m: string) => void): Promise<void> {
  if (credential) return;
  // 1. Mint a short-lived bootstrap token (open endpoint).
  const tok = await fetch(`${VELOS}/auth/v1/tokens`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ ttlSeconds: 86400 }),
  }).then((r) => r.json());
  const bootstrap = `${tok.tokenId}.${tok.secret}`;

  // 2. Exchange it for a durable worker credential under a dashboard identity.
  const reg = await fetch(`${VELOS}/auth/v1/register`, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${bootstrap}` },
    body: JSON.stringify({
      name: "velos-dashboard",
      capacity: {},
      addresses: [],
      containerRuntimeVersion: "dashboard",
    }),
  }).then((r) => r.json());
  credential = reg.token;

  // 3. The credential is stored independently of the Worker object, so delete
  //    the dashboard's Worker doc to keep it out of the workers list.
  await fetch(`${VELOS}/api/v1/workers/velos-dashboard`, {
    method: "DELETE",
    headers: { authorization: `Bearer ${credential}` },
  }).catch(() => {});

  log(`velos: authenticated as velos-dashboard (${VELOS})`);
}

function velosAuth(): Plugin {
  return {
    name: "velos-auth",
    async configureServer(server) {
      try {
        await ensureCredential((m) => server.config.logger.info(`  ➜  ${m}`));
      } catch (e) {
        server.config.logger.error(`velos auth failed (is the apiserver up at ${VELOS}?): ${e}`);
      }
    },
  };
}

export default defineConfig({
  plugins: [react(), tailwindcss(), velosAuth()],
  server: {
    port: 5173,
    proxy: {
      "/velos": {
        target: VELOS,
        changeOrigin: true,
        rewrite: (p) => p.replace(/^\/velos/, ""),
        configure(proxy) {
          proxy.on("proxyReq", (proxyReq) => {
            if (credential) proxyReq.setHeader("authorization", `Bearer ${credential}`);
          });
        },
      },
    },
  },
});
