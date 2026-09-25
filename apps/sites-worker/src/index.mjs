import { handleAccountRoute } from "./accounts.mjs";
import { handleActivitySearchRoute } from "./activities.mjs";
import { handleActivityImportMappingRoute } from "./activity-import-mappings.mjs";
import {
  handleCheckActivitiesImport,
  handleCheckExistingDuplicates,
  handleImportActivities,
  handleParseCsv,
} from "./activity-imports.mjs";
import { handleAssetRoute } from "./assets.mjs";
import { handleMcpRoute, handleSpendingCategoryRoute } from "./mcp.mjs";
import { handlePortfolioRoute } from "./portfolio.mjs";
import { handleSettingsRoute } from "./settings.mjs";
import { handleTaxonomyRoute } from "./taxonomies.mjs";
import { handleWealthfolioMigrationRoute } from "./wealthfolio-migration.mjs";

const API_PREFIX = "/api/v1";

function json(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json; charset=utf-8", "cache-control": "no-store" },
  });
}

function methodNotAllowed() {
  return json({ message: "Method not allowed" }, 405);
}

async function serveStatic(request, env) {
  const assets = env.ASSETS;
  if (!assets || typeof assets.fetch !== "function") {
    return new Response("The Sites static asset binding is unavailable.", { status: 503 });
  }

  const response = await assets.fetch(request);
  if (response.status !== 404 || request.method !== "GET") return response;

  const accept = request.headers.get("accept") ?? "";
  if (!accept.includes("text/html")) return response;
  const url = new URL(request.url);
  return assets.fetch(new Request(new URL("/index.html", url.origin), request));
}

async function handleApi(request, env, pathname) {
  const ownerId = request.headers.get("oai-authenticated-user-id")?.trim();
  if (!ownerId) return json({ message: "A private signed-in Sites session is required." }, 401);

  const route = pathname.slice(API_PREFIX.length) || "/";
  // These empty read models have no corresponding tables or provider services
  // in the Sites profile. Returning their actual empty state keeps the shared
  // frontend from treating absent optional features as server failures.
  if (request.method === "GET" && ["/goals", "/portfolios", "/assets/logos", "/platforms"].includes(route)) {
    return json([]);
  }
  if (request.method === "GET" && route === "/spending/settings") {
    return json({ enabled: false, accountIds: [], excludedCategoryIds: [] });
  }
  if (request.method === "GET" && ["/providers", "/exchanges", "/custom-providers"].includes(route)) {
    // Sites has no market-data provider registry or exchange catalog.
    return json([]);
  }
  if (request.method === "GET" && route === "/health/status") {
    return json({ overallSeverity: "INFO", issueCounts: {}, issues: [], checkedAt: new Date().toISOString(), isStale: true });
  }
  if (route === "/auth/status" && request.method === "GET") {
    return json({ requiresPassword: false, oidcEnabled: false });
  }
  if (route === "/app/info" && request.method === "GET") {
    return json({ name: "Wealthfolio", version: "gpt-sites-test", platform: "sites" });
  }
  if (route.startsWith("/settings")) {
    const response = await handleSettingsRoute(request, route, ownerId, env);
    if (response) return response;
  }
  if (route === "/accounts" || route.startsWith("/accounts/")) {
    const response = await handleAccountRoute(request, route, ownerId, env);
    if (response) return response;
  }
  const activitySearchResponse = await handleActivitySearchRoute(request, route, ownerId, env);
  if (activitySearchResponse) return activitySearchResponse;
  const importMappingResponse = await handleActivityImportMappingRoute(request, route, ownerId, env);
  if (importMappingResponse) return importMappingResponse;
  const assetResponse = await handleAssetRoute(request, route, ownerId, env);
  if (assetResponse) return assetResponse;
  if (route.startsWith("/taxonomies")) {
    const response = await handleTaxonomyRoute(request, route, ownerId, env);
    if (response) return response;
  }
  if (route.startsWith("/activities/import/parse")) {
    if (request.method === "POST") return handleParseCsv(request);
    return methodNotAllowed();
  }
  if (route === "/activities/import/check" && request.method === "POST") {
    return handleCheckActivitiesImport(request, env);
  }
  if (route === "/activities/import" && request.method === "POST") {
    return handleImportActivities(request, env);
  }
  if (route === "/activities/import/check-duplicates" && request.method === "POST") {
    return handleCheckExistingDuplicates(request, env);
  }
  if (route.startsWith("/sites/migration/")) {
    const response = await handleWealthfolioMigrationRoute(request, route, ownerId, env);
    if (response) return response;
  }
  if (route.startsWith("/sites/mcp/")) {
    const response = await handleMcpRoute(request, route, ownerId, env);
    if (response) return response;
  }
  if (route.startsWith("/spending/")) {
    const response = await handleSpendingCategoryRoute(request, route, ownerId, env);
    if (response) return response;
  }

  const portfolioResponse = await handlePortfolioRoute(request, route, ownerId, env);
  if (portfolioResponse) return portfolioResponse;

  return json({ message: "This Wealthfolio feature is not available in the Sites test port yet." }, 501);
}

export default {
  async fetch(request, env, ctx) {
    void ctx;
    const url = new URL(request.url);
    if (url.pathname === API_PREFIX || url.pathname.startsWith(`${API_PREFIX}/`)) {
      try {
        return await handleApi(request, env, url.pathname);
      } catch {
        return json({ message: "The request could not be completed." }, 500);
      }
    }
    return serveStatic(request, env);
  },
};

export { handleApi };
