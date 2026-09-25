import { describe, expect, it, vi } from "vitest";

import { registerSitesMcpTools, SITES_MCP_ROUTES, type SitesMcpDocument } from "./index";

type RegisteredTool = Parameters<NonNullable<SitesMcpDocument["modelContext"]>["registerTool"]>[0];

function makeDocument() {
  const tools: RegisteredTool[] = [];
  const registerTool = vi.fn((tool: RegisteredTool) => {
    tools.push(tool);
  });
  return {
    document: { modelContext: { registerTool } } satisfies SitesMcpDocument,
    registerTool,
    tools,
  };
}

function makeFetch(response: unknown = { ok: true }) {
  const fetcher = vi.fn(
    async () =>
      new Response(JSON.stringify(response), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
  );
  return fetcher;
}

function getTool(tools: RegisteredTool[], name: string): RegisteredTool {
  const tool = tools.find((candidate) => candidate.name === name);
  if (!tool) {
    throw new Error(`Missing registered tool: ${name}`);
  }
  return tool;
}

describe("Sites WebMCP tools", () => {
  it("feature detects document.modelContext and registers all tools", async () => {
    const { document, registerTool, tools } = makeDocument();
    const registration = await registerSitesMcpTools({ document, fetch: makeFetch() });

    expect(registration).not.toBeNull();
    expect(registerTool).toHaveBeenCalledTimes(5);
    expect(tools.map((tool) => tool.name)).toEqual([
      "commit_category_assignments",
      "commit_categorization_rule",
      "create_account",
      "list_categorization_context",
      "propose_transaction_categories",
    ]);
    expect(tools.slice(0, 3).map((tool) => tool.annotations.readOnlyHint)).toEqual([
      false,
      false,
      false,
    ]);
    expect(tools.slice(3).map((tool) => tool.annotations.readOnlyHint)).toEqual([true, true]);
    expect(tools.every((tool) => tool.inputSchema.type === "object")).toBe(true);
  });

  it("mirrors the reviewed required fields and enum constraints", async () => {
    const { document, tools } = makeDocument();
    await registerSitesMcpTools({ document, fetch: makeFetch() });

    expect(getTool(tools, "commit_category_assignments").inputSchema).toMatchObject({
      required: ["assignments"],
      properties: { assignments: { maxItems: 1_000 } },
    });
    expect(getTool(tools, "commit_categorization_rule").inputSchema).toMatchObject({
      required: ["pattern", "taxonomyId", "categoryKey"],
      properties: {
        matchType: { enum: ["contains", "starts_with", "exact", "regex"] },
      },
    });
    expect(getTool(tools, "create_account").inputSchema).toMatchObject({
      required: ["name", "accountType", "currency"],
      properties: {
        accountType: {
          enum: ["SECURITIES", "CASH", "CREDIT_CARD", "CRYPTOCURRENCY"],
        },
      },
    });
    expect(getTool(tools, "list_categorization_context").inputSchema).toMatchObject({
      properties: {
        status: { enum: ["uncategorized", "all", "needs_review"] },
        limit: { minimum: 1, maximum: 100 },
      },
    });
    expect(getTool(tools, "propose_transaction_categories").inputSchema).toMatchObject({
      properties: {
        aiProposals: {
          items: {
            required: ["activityId", "taxonomyId", "categoryKey"],
          },
        },
      },
    });
  });

  it("posts valid camelCase inputs and returns JSON results", async () => {
    const { document, tools } = makeDocument();
    const response = { applied: [{ activityId: "activity-1", categoryId: "category-1" }] };
    const fetcher = makeFetch(response);
    await registerSitesMcpTools({ document, fetch: fetcher });

    const result = await getTool(tools, "commit_category_assignments").execute({
      assignments: [
        { activityId: "activity-1", taxonomyId: "taxonomy-1", categoryKey: "groceries" },
      ],
    });

    expect(result).toEqual(response);
    expect(fetcher).toHaveBeenCalledWith(
      SITES_MCP_ROUTES.commitCategoryAssignments,
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({
          assignments: [
            { activityId: "activity-1", taxonomyId: "taxonomy-1", categoryKey: "groceries" },
          ],
        }),
      }),
    );
  });

  it("posts valid rule and account inputs to their exact routes", async () => {
    const { document, tools } = makeDocument();
    const fetcher = makeFetch({ rule: { id: "rule-1" } });
    await registerSitesMcpTools({ document, fetch: fetcher });

    await getTool(tools, "commit_categorization_rule").execute({
      pattern: "T&T",
      taxonomyId: "taxonomy-1",
      categoryKey: "groceries",
      matchType: "contains",
    });
    await getTool(tools, "create_account").execute({
      name: "Everyday Cash",
      accountType: "cash",
      currency: "cad",
      isDefault: false,
    });

    expect(fetcher).toHaveBeenNthCalledWith(
      1,
      SITES_MCP_ROUTES.commitCategorizationRule,
      expect.objectContaining({ method: "POST" }),
    );
    expect(fetcher).toHaveBeenNthCalledWith(
      2,
      SITES_MCP_ROUTES.createAccount,
      expect.objectContaining({ method: "POST" }),
    );
  });

  it("posts valid read and proposal inputs to their exact routes", async () => {
    const { document, tools } = makeDocument();
    const fetcher = makeFetch({ proposals: [] });
    await registerSitesMcpTools({ document, fetch: fetcher });

    await getTool(tools, "list_categorization_context").execute({
      status: "uncategorized",
      limit: 25,
    });
    await getTool(tools, "propose_transaction_categories").execute({
      status: "uncategorized",
      aiProposals: [],
    });

    expect(fetcher).toHaveBeenNthCalledWith(
      1,
      SITES_MCP_ROUTES.listCategorizationContext,
      expect.objectContaining({ method: "POST" }),
    );
    expect(fetcher).toHaveBeenNthCalledWith(
      2,
      SITES_MCP_ROUTES.proposeTransactionCategories,
      expect.objectContaining({ method: "POST" }),
    );
  });

  it("rejects invalid inputs before making a request", async () => {
    const { document, tools } = makeDocument();
    const fetcher = makeFetch();
    await registerSitesMcpTools({ document, fetch: fetcher });

    await expect(
      getTool(tools, "commit_category_assignments").execute({ assignments: [{}] }),
    ).rejects.toThrow("activityId");
    await expect(
      getTool(tools, "commit_categorization_rule").execute({
        pattern: "T&T",
        taxonomyId: "taxonomy-1",
        categoryKey: "groceries",
        matchType: "unsupported",
      }),
    ).rejects.toThrow("matchType");
    await expect(
      getTool(tools, "create_account").execute({
        name: "Cash",
        accountType: "SAVINGS",
        currency: "CAD",
      }),
    ).rejects.toThrow("accountType");
    await expect(
      getTool(tools, "list_categorization_context").execute({ limit: 0 }),
    ).rejects.toThrow("limit");
    await expect(
      getTool(tools, "propose_transaction_categories").execute({
        aiProposals: [
          {
            activityId: "activity-1",
            taxonomyId: "taxonomy-1",
            categoryKey: "food",
            confidence: 2,
          },
        ],
      }),
    ).rejects.toThrow("confidence");
    expect(fetcher).not.toHaveBeenCalled();
  });

  it("does nothing when WebMCP is unavailable", async () => {
    await expect(registerSitesMcpTools({ document: {} })).resolves.toBeNull();
  });
});
