const COMMIT_ASSIGNMENTS_ROUTE = "/api/v1/sites/mcp/commit-category-assignments";
const COMMIT_RULE_ROUTE = "/api/v1/sites/mcp/commit-categorization-rule";
const CREATE_ACCOUNT_ROUTE = "/api/v1/sites/mcp/create-account";
const LIST_CONTEXT_ROUTE = "/api/v1/sites/mcp/list-categorization-context";
const PROPOSE_CATEGORIES_ROUTE = "/api/v1/sites/mcp/propose-transaction-categories";
const MAX_COMMIT_ASSIGNMENTS = 1_000;
const MAX_CATEGORIZATION_LIMIT = 100;
const MAX_RULE_PATTERN_BYTES = 512;
const RULE_MATCH_TYPES = ["contains", "starts_with", "exact", "regex"] as const;
const ACCOUNT_TYPES = ["SECURITIES", "CASH", "CREDIT_CARD", "CRYPTOCURRENCY"] as const;

type JsonPrimitive = string | number | boolean | null;
type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue };

type FetchLike = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

interface ModelContextToolOptions {
  signal?: AbortSignal;
}

interface ModelContextTool {
  name: string;
  description: string;
  inputSchema: Record<string, unknown>;
  annotations: { readOnlyHint: boolean };
  execute: (input: unknown, options?: ModelContextToolOptions) => Promise<JsonValue>;
}

interface ModelContextLike {
  registerTool(tool: ModelContextTool, options?: ModelContextToolOptions): Promise<void> | void;
}

export interface SitesMcpDocument {
  modelContext?: ModelContextLike;
}

export interface SitesMcpRegistration {
  unregister: () => void;
}

export interface RegisterSitesMcpToolsOptions {
  document?: SitesMcpDocument;
  fetch?: FetchLike;
}

export const SITES_MCP_ROUTES = {
  commitCategoryAssignments: COMMIT_ASSIGNMENTS_ROUTE,
  commitCategorizationRule: COMMIT_RULE_ROUTE,
  createAccount: CREATE_ACCOUNT_ROUTE,
  listCategorizationContext: LIST_CONTEXT_ROUTE,
  proposeTransactionCategories: PROPOSE_CATEGORIES_ROUTE,
} as const;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function requireObject(input: unknown, toolName: string): Record<string, unknown> {
  if (!isRecord(input)) {
    throw new TypeError(`${toolName} input must be an object`);
  }
  return input;
}

function requireNonEmptyString(
  input: Record<string, unknown>,
  field: string,
  toolName: string,
): string {
  const value = input[field];
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new TypeError(`${toolName} requires a non-empty ${field}`);
  }
  return value;
}

function requireOptionalString(
  input: Record<string, unknown>,
  field: string,
  toolName: string,
): void {
  if (input[field] !== undefined && typeof input[field] !== "string") {
    throw new TypeError(`${toolName} ${field} must be a string`);
  }
}

const CATEGORIZATION_STATUSES = ["uncategorized", "all", "needs_review"] as const;

function requireStringArray(
  input: Record<string, unknown>,
  field: string,
  toolName: string,
  maxLength?: number,
): void {
  const value = input[field];
  if (
    !Array.isArray(value) ||
    value.some((item) => typeof item !== "string" || item.trim() === "")
  ) {
    throw new TypeError(`${toolName} ${field} must be an array of non-empty strings`);
  }
  if (maxLength !== undefined && value.length > maxLength) {
    throw new TypeError(`${toolName} ${field} accepts at most ${maxLength} items`);
  }
}

function validateCategorizationFilters(
  input: unknown,
  toolName: string,
  includeAiProposals: boolean,
): Record<string, unknown> {
  const object = requireObject(input, toolName);
  if (object.activityIds !== undefined) {
    requireStringArray(object, "activityIds", toolName, MAX_CATEGORIZATION_LIMIT);
  }
  if (object.accountIds !== undefined) {
    requireStringArray(object, "accountIds", toolName);
  }
  if (object.status !== undefined) {
    if (
      typeof object.status !== "string" ||
      !CATEGORIZATION_STATUSES.includes(object.status as (typeof CATEGORIZATION_STATUSES)[number])
    ) {
      throw new TypeError(`${toolName} status is unsupported`);
    }
  }
  for (const field of ["startDate", "endDate"]) {
    if (object[field] !== undefined) {
      requireNonEmptyString(object, field, toolName);
    }
  }
  if (object.limit !== undefined) {
    if (
      typeof object.limit !== "number" ||
      !Number.isInteger(object.limit) ||
      object.limit < 1 ||
      object.limit > MAX_CATEGORIZATION_LIMIT
    ) {
      throw new TypeError(
        `${toolName} limit must be an integer from 1 to ${MAX_CATEGORIZATION_LIMIT}`,
      );
    }
  }
  if (includeAiProposals && object.aiProposals !== undefined) {
    if (!Array.isArray(object.aiProposals)) {
      throw new TypeError(`${toolName} aiProposals must be an array`);
    }
    object.aiProposals.forEach((proposal, index) => {
      const item = requireObject(proposal, `${toolName} aiProposal ${index}`);
      requireNonEmptyString(item, "activityId", toolName);
      requireNonEmptyString(item, "taxonomyId", toolName);
      requireNonEmptyString(item, "categoryKey", toolName);
      requireOptionalString(item, "reason", toolName);
      if (item.confidence !== undefined) {
        if (
          typeof item.confidence !== "number" ||
          !Number.isFinite(item.confidence) ||
          item.confidence < 0 ||
          item.confidence > 1
        ) {
          throw new TypeError(`${toolName} aiProposal confidence must be between 0 and 1`);
        }
      }
    });
  }
  return object;
}

function validateCategoryAssignments(input: unknown): Record<string, unknown> {
  const toolName = "commit_category_assignments";
  const object = requireObject(input, toolName);
  const assignments = object.assignments;
  if (!Array.isArray(assignments)) {
    throw new TypeError(`${toolName} requires assignments to be an array`);
  }
  if (assignments.length > MAX_COMMIT_ASSIGNMENTS) {
    throw new TypeError(`${toolName} accepts at most ${MAX_COMMIT_ASSIGNMENTS} assignments`);
  }
  assignments.forEach((assignment, index) => {
    const item = requireObject(assignment, `${toolName} assignment ${index}`);
    requireNonEmptyString(item, "activityId", toolName);
    requireNonEmptyString(item, "taxonomyId", toolName);
    requireNonEmptyString(item, "categoryKey", toolName);
  });
  return object;
}

function validateCategorizationRule(input: unknown): Record<string, unknown> {
  const toolName = "commit_categorization_rule";
  const object = requireObject(input, toolName);
  const pattern = requireNonEmptyString(object, "pattern", toolName);
  if (new TextEncoder().encode(pattern).length > MAX_RULE_PATTERN_BYTES) {
    throw new TypeError(`${toolName} pattern is too long`);
  }
  requireNonEmptyString(object, "taxonomyId", toolName);
  requireNonEmptyString(object, "categoryKey", toolName);
  requireOptionalString(object, "name", toolName);
  requireOptionalString(object, "activityType", toolName);
  requireOptionalString(object, "accountId", toolName);

  if (object.matchType !== undefined) {
    if (
      typeof object.matchType !== "string" ||
      !RULE_MATCH_TYPES.includes(object.matchType as (typeof RULE_MATCH_TYPES)[number])
    ) {
      throw new TypeError(`${toolName} matchType is unsupported`);
    }
  }
  return object;
}

function validateCreateAccount(input: unknown): Record<string, unknown> {
  const toolName = "create_account";
  const object = requireObject(input, toolName);
  requireNonEmptyString(object, "name", toolName);
  const accountType = requireNonEmptyString(object, "accountType", toolName);
  requireNonEmptyString(object, "currency", toolName);
  requireOptionalString(object, "group", toolName);

  if (!ACCOUNT_TYPES.includes(accountType.trim().toUpperCase() as (typeof ACCOUNT_TYPES)[number])) {
    throw new TypeError(`${toolName} accountType is unsupported`);
  }
  if (object.isDefault !== undefined && typeof object.isDefault !== "boolean") {
    throw new TypeError(`${toolName} isDefault must be a boolean`);
  }
  return object;
}

function isJsonValue(value: unknown): value is JsonValue {
  if (value === null || typeof value === "string" || typeof value === "boolean") {
    return true;
  }
  if (typeof value === "number") {
    return Number.isFinite(value);
  }
  if (Array.isArray(value)) {
    return value.every(isJsonValue);
  }
  return isRecord(value) && Object.values(value).every(isJsonValue);
}

async function postJson(
  route: string,
  input: Record<string, unknown>,
  fetcher: FetchLike | undefined,
  signal: AbortSignal,
  toolName: string,
): Promise<JsonValue> {
  if (!fetcher) {
    throw new Error(`${toolName} requires the Fetch API`);
  }
  const response = await fetcher(route, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    credentials: "same-origin",
    body: JSON.stringify(input),
    signal,
  });
  if (!response.ok) {
    throw new Error(`${toolName} request failed with status ${response.status}`);
  }
  let result: unknown;
  try {
    result = await response.json();
  } catch {
    throw new Error(`${toolName} returned invalid JSON`);
  }
  if (!isJsonValue(result)) {
    throw new Error(`${toolName} returned a non-JSON result`);
  }
  return result;
}

function createTools(
  fetcher: FetchLike | undefined,
  registrationSignal: AbortSignal,
): ModelContextTool[] {
  return [
    {
      name: "commit_category_assignments",
      description:
        "Persist reviewed category assignments as real activity categories. Pass each proposal's activityId, taxonomyId, and categoryKey. This MUTATES data; call only after review and confirmation. At most 1000 assignments per call.",
      inputSchema: {
        type: "object",
        properties: {
          assignments: {
            type: "array",
            description: "Reviewed category assignments to persist.",
            maxItems: MAX_COMMIT_ASSIGNMENTS,
            items: {
              type: "object",
              properties: {
                activityId: { type: "string" },
                taxonomyId: { type: "string" },
                categoryKey: {
                  type: "string",
                  description: 'Category key from the taxonomy (e.g. "groceries").',
                },
              },
              required: ["activityId", "taxonomyId", "categoryKey"],
            },
          },
        },
        required: ["assignments"],
      },
      annotations: { readOnlyHint: false },
      execute: async (input, options) =>
        postJson(
          COMMIT_ASSIGNMENTS_ROUTE,
          validateCategoryAssignments(input),
          fetcher,
          options?.signal ?? registrationSignal,
          "commit_category_assignments",
        ),
    },
    {
      name: "commit_categorization_rule",
      description:
        "Persist a categorization rule directly without a confirmation widget. Pass pattern, taxonomyId, and categoryKey; matchType defaults to contains and accountId optionally scopes the rule. This MUTATES data; the rule is saved immediately.",
      inputSchema: {
        type: "object",
        properties: {
          name: {
            type: "string",
            description:
              'Short rule name shown in settings. Default: derive from pattern, e.g. "T&T → Groceries".',
          },
          pattern: {
            type: "string",
            description:
              "Substring/pattern matched against transaction notes. contains/starts_with/exact are case-insensitive; regex is a Rust regex and is case-sensitive unless it uses an inline flag like (?i).",
          },
          matchType: {
            type: "string",
            enum: [...RULE_MATCH_TYPES],
            description: 'Default "contains". Use stricter modes only if needed.',
          },
          categoryKey: {
            type: "string",
            description: 'Category key from the activity-scope taxonomies (e.g. "groceries").',
          },
          taxonomyId: {
            type: "string",
            description:
              "Taxonomy ID containing categoryKey. Required because category keys are taxonomy-scoped.",
          },
          activityType: {
            type: "string",
            description: "Optional activity-type narrowing (e.g. WITHDRAWAL). Usually omit.",
          },
          accountId: {
            type: "string",
            description:
              "Optional account ID to scope the rule to one account. Omit for a global rule.",
          },
        },
        required: ["pattern", "taxonomyId", "categoryKey"],
      },
      annotations: { readOnlyHint: false },
      execute: async (input, options) =>
        postJson(
          COMMIT_RULE_ROUTE,
          validateCategorizationRule(input),
          fetcher,
          options?.signal ?? registrationSignal,
          "commit_categorization_rule",
        ),
    },
    {
      name: "create_account",
      description:
        "Create a Wealthfolio account (SECURITIES, CASH, CREDIT_CARD, or CRYPTOCURRENCY). Pass name, accountType, and currency; group and isDefault are optional. This MUTATES data; the account is created immediately.",
      inputSchema: {
        type: "object",
        properties: {
          name: { type: "string", description: "Display name for the new account." },
          accountType: {
            type: "string",
            enum: [...ACCOUNT_TYPES],
            description: "Account type (case-insensitive).",
          },
          currency: { type: "string", description: 'ISO currency code, e.g. "CAD".' },
          group: { type: "string", description: "Optional grouping label." },
          isDefault: {
            type: "boolean",
            description: "Make this the default account. Defaults to false.",
          },
        },
        required: ["name", "accountType", "currency"],
      },
      annotations: { readOnlyHint: false },
      execute: async (input, options) =>
        postJson(
          CREATE_ACCOUNT_ROUTE,
          validateCreateAccount(input),
          fetcher,
          options?.signal ?? registrationSignal,
          "create_account",
        ),
    },
    {
      name: "list_categorization_context",
      description:
        "Prerequisite for propose_transaction_categories. Returns activity-scope taxonomies, recent few-shot examples, and cash transactions needing AI categorization. Rows matched by rules or same-payee history are excluded from unproposed but are not applied. This is read-only context; call propose_transaction_categories with the same filters to render the review draft.",
      inputSchema: {
        type: "object",
        properties: {
          activityIds: {
            type: "array",
            items: { type: "string" },
            description: "Optional explicit set of activity IDs.",
          },
          accountIds: {
            type: "array",
            items: { type: "string" },
            description: "OMIT unless the user names a specific account by exact name or ID.",
          },
          status: {
            type: "string",
            enum: [...CATEGORIZATION_STATUSES],
            description: "Default: uncategorized.",
          },
          startDate: { type: "string", description: "Inclusive ISO 8601 lower bound." },
          endDate: { type: "string", description: "Inclusive ISO 8601 upper bound." },
          limit: {
            type: "integer",
            minimum: 1,
            maximum: MAX_CATEGORIZATION_LIMIT,
            description: "Max rows. Default 100.",
          },
        },
      },
      annotations: { readOnlyHint: true },
      execute: async (input, options) =>
        postJson(
          LIST_CONTEXT_ROUTE,
          validateCategorizationFilters(input, "list_categorization_context", false),
          fetcher,
          options?.signal ?? registrationSignal,
          "list_categorization_context",
        ),
    },
    {
      name: "propose_transaction_categories",
      description:
        "Generate a read-only categorization draft for user review. Run list_categorization_context first, then pass the same filters and inferred aiProposals for unproposed rows. This creates proposals for review and does not apply categories or mutate data.",
      inputSchema: {
        type: "object",
        properties: {
          activityIds: {
            type: "array",
            items: { type: "string" },
            description:
              "Optional explicit set of activity IDs to propose for. Intersected with the other filters.",
          },
          accountIds: {
            type: "array",
            items: { type: "string" },
            description: "OMIT unless the user names a specific account by exact name or ID.",
          },
          status: {
            type: "string",
            enum: [...CATEGORIZATION_STATUSES],
            description: "Default: uncategorized.",
          },
          startDate: { type: "string", description: "Inclusive ISO 8601 lower bound." },
          endDate: { type: "string", description: "Inclusive ISO 8601 upper bound." },
          limit: {
            type: "integer",
            minimum: 1,
            maximum: MAX_CATEGORIZATION_LIMIT,
            description:
              "Max rows to propose. Default 100 (also the cap). When summary.total equals the limit, more uncategorized rows likely remain.",
          },
          aiProposals: {
            type: "array",
            description:
              "Inferred categories for rows returned as unproposed by list_categorization_context. Pass [] when that context returned needsAiJudgement = 0.",
            items: {
              type: "object",
              properties: {
                activityId: { type: "string" },
                taxonomyId: { type: "string" },
                categoryKey: { type: "string" },
                confidence: { type: "number", minimum: 0, maximum: 1 },
                reason: { type: "string" },
              },
              required: ["activityId", "taxonomyId", "categoryKey"],
            },
          },
        },
      },
      annotations: { readOnlyHint: true },
      execute: async (input, options) =>
        postJson(
          PROPOSE_CATEGORIES_ROUTE,
          validateCategorizationFilters(input, "propose_transaction_categories", true),
          fetcher,
          options?.signal ?? registrationSignal,
          "propose_transaction_categories",
        ),
    },
  ];
}

export async function registerSitesMcpTools(
  options: RegisterSitesMcpToolsOptions = {},
): Promise<SitesMcpRegistration | null> {
  const document: SitesMcpDocument | undefined =
    options.document ??
    (typeof globalThis.document === "undefined"
      ? undefined
      : (globalThis.document as SitesMcpDocument));
  const modelContext = document?.modelContext;
  if (!modelContext) {
    return null;
  }

  const fetcher =
    options.fetch ??
    (typeof globalThis.fetch === "function" ? globalThis.fetch.bind(globalThis) : undefined);
  const controller = new AbortController();
  try {
    await Promise.all(
      createTools(fetcher, controller.signal).map((tool) =>
        modelContext.registerTool(tool, { signal: controller.signal }),
      ),
    );
  } catch (error) {
    controller.abort();
    throw error;
  }
  return { unregister: () => controller.abort() };
}
