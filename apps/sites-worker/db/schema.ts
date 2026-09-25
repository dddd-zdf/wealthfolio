import { sql } from "drizzle-orm";
import { index, integer, primaryKey, sqliteTable, text, uniqueIndex } from "drizzle-orm/sqlite-core";

export const accounts = sqliteTable(
  "accounts",
  {
    id: text("id").primaryKey(),
    ownerId: text("owner_id").notNull(),
    name: text("name").notNull(),
    accountType: text("account_type").notNull(),
    currency: text("currency").notNull(),
    groupName: text("group_name"),
    trackingMode: text("tracking_mode").notNull().default("NOT_SET"),
    meta: text("meta"),
    isDefault: integer("is_default").notNull().default(0),
    isActive: integer("is_active").notNull().default(1),
    isArchived: integer("is_archived").notNull().default(0),
    createdAt: text("created_at").notNull().default(sql`CURRENT_TIMESTAMP`),
    updatedAt: text("updated_at").notNull().default(sql`CURRENT_TIMESTAMP`),
  },
  (table) => [index("accounts_owner_idx").on(table.ownerId)],
);

export const userSettings = sqliteTable("user_settings", {
  ownerId: text("owner_id").primaryKey(),
  settingsJson: text("settings_json").notNull(),
  updatedAt: text("updated_at").notNull().default(sql`CURRENT_TIMESTAMP`),
});

export const activityRecords = sqliteTable(
  "activity_records",
  {
    id: text("id").primaryKey(),
    ownerId: text("owner_id").notNull(),
    accountId: text("account_id").notNull(),
    activityDate: text("activity_date").notNull(),
    assetId: text("asset_id"),
    symbol: text("symbol"),
    activityType: text("activity_type").notNull(),
    quantity: text("quantity"),
    unitPrice: text("unit_price"),
    currency: text("currency"),
    fee: text("fee"),
    tax: text("tax"),
    amount: text("amount"),
    idempotencyKey: text("idempotency_key"),
    payloadJson: text("payload_json").notNull(),
    importRunId: text("import_run_id"),
    createdAt: text("created_at").notNull(),
  },
  (table) => [
    index("activity_owner_date_idx").on(table.ownerId, table.activityDate),
    index("activity_owner_account_idx").on(table.ownerId, table.accountId),
    uniqueIndex("activity_owner_idempotency_uq").on(table.ownerId, table.idempotencyKey),
  ],
);

export const assetQuotes = sqliteTable(
  "asset_quotes",
  {
    id: text("id").primaryKey(),
    ownerId: text("owner_id").notNull(),
    assetId: text("asset_id"),
    symbol: text("symbol"),
    quoteDate: text("quote_date").notNull(),
    price: text("price").notNull(),
    currency: text("currency").notNull(),
  },
  (table) => [
    index("quotes_owner_date_idx").on(table.ownerId, table.quoteDate),
    index("quotes_owner_asset_idx").on(table.ownerId, table.assetId),
  ],
);

export const taxonomies = sqliteTable(
  "taxonomies",
  {
    id: text("id").notNull(),
    ownerId: text("owner_id").notNull(),
    name: text("name").notNull(),
    color: text("color").notNull().default("#B89A4C"),
    description: text("description"),
    scope: text("scope").notNull().default("activity"),
    isSingleSelect: integer("is_single_select").notNull().default(1),
    sortOrder: integer("sort_order").notNull().default(0),
    isDefault: integer("is_default").notNull().default(0),
    createdAt: text("created_at").notNull().default(sql`CURRENT_TIMESTAMP`),
  },
  (table) => [
    primaryKey({ columns: [table.ownerId, table.id] }),
    index("taxonomies_owner_scope_idx").on(table.ownerId, table.scope),
  ],
);

export const categories = sqliteTable(
  "categories",
  {
    id: text("id").notNull(),
    ownerId: text("owner_id").notNull(),
    taxonomyId: text("taxonomy_id").notNull(),
    categoryKey: text("category_key").notNull(),
    name: text("name").notNull(),
    parentId: text("parent_id"),
    color: text("color").notNull().default("#B89A4C"),
    description: text("description"),
    icon: text("icon"),
    position: integer("position").notNull().default(0),
    isActive: integer("is_active").notNull().default(1),
    createdAt: text("created_at").notNull().default(sql`CURRENT_TIMESTAMP`),
    updatedAt: text("updated_at").notNull().default(sql`CURRENT_TIMESTAMP`),
  },
  (table) => [
    primaryKey({ columns: [table.ownerId, table.id] }),
    uniqueIndex("categories_owner_taxonomy_key_uq").on(table.ownerId, table.taxonomyId, table.categoryKey),
    index("categories_owner_taxonomy_idx").on(table.ownerId, table.taxonomyId),
  ],
);

export const activityCategoryAssignments = sqliteTable(
  "activity_category_assignments",
  {
    id: text("id").primaryKey(),
    ownerId: text("owner_id").notNull(),
    activityId: text("activity_id").notNull(),
    taxonomyId: text("taxonomy_id").notNull(),
    categoryId: text("category_id").notNull(),
    createdAt: text("created_at").notNull().default(sql`CURRENT_TIMESTAMP`),
  },
  (table) => [
    uniqueIndex("activity_category_owner_uq").on(table.ownerId, table.activityId, table.taxonomyId),
    index("activity_category_owner_activity_idx").on(table.ownerId, table.activityId),
  ],
);

export const categorizationRules = sqliteTable(
  "categorization_rules",
  {
    id: text("id").primaryKey(),
    ownerId: text("owner_id").notNull(),
    name: text("name").notNull(),
    pattern: text("pattern").notNull(),
    matchType: text("match_type").notNull().default("contains"),
    taxonomyId: text("taxonomy_id").notNull(),
    categoryId: text("category_id").notNull(),
    activityType: text("activity_type"),
    accountId: text("account_id"),
    isGlobal: integer("is_global").notNull().default(0),
    isEnabled: integer("is_enabled").notNull().default(1),
    createdAt: text("created_at").notNull().default(sql`CURRENT_TIMESTAMP`),
  },
  (table) => [index("rules_owner_enabled_idx").on(table.ownerId, table.isEnabled)],
);

export const mcpAuditLogs = sqliteTable(
  "mcp_audit_logs",
  {
    id: text("id").primaryKey(),
    ownerId: text("owner_id").notNull(),
    action: text("action").notNull(),
    resultCount: integer("result_count").notNull().default(0),
    createdAt: text("created_at").notNull().default(sql`CURRENT_TIMESTAMP`),
  },
  (table) => [index("mcp_audit_owner_time_idx").on(table.ownerId, table.createdAt)],
);
