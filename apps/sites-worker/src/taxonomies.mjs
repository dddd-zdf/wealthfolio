const TAXONOMY_SEEDS = [
  { id: "spending_categories", name: "Spending Categories", color: "#B0552E", description: "Expense categories", scope: "activity", sortOrder: 200 },
  { id: "income_sources", name: "Income Sources", color: "#5A7A3E", description: "Income categories", scope: "activity", sortOrder: 210 },
  { id: "savings_categories", name: "Savings", color: "#6B8E54", description: "Savings and investment transfers", scope: "activity", sortOrder: 220 },
];

const CATEGORY_SEEDS = [
  ["cat_housing", "spending_categories", "housing", "Housing", null, "#A35742", "Home", 1],
  ["cat_groceries", "spending_categories", "groceries", "Groceries", null, "#355C4C", "ShoppingCart", 2],
  ["cat_food", "spending_categories", "food", "Food & Dining", null, "#B89A4C", "UtensilsCrossed", 3],
  ["cat_transport", "spending_categories", "transport", "Transportation", null, "#7B96C9", "Car", 4],
  ["cat_shopping", "spending_categories", "shopping", "Shopping", null, "#8E7CB3", "ShoppingBag", 5],
  ["cat_entertainment", "spending_categories", "entertainment", "Entertainment", null, "#B0552E", "Film", 6],
  ["cat_health", "spending_categories", "health", "Health & Wellness", null, "#6B8E54", "Heart", 7],
  ["cat_bills", "spending_categories", "bills", "Bills & Utilities", null, "#4F6B92", "FileText", 8],
  ["cat_personal", "spending_categories", "personal", "Personal Care", null, "#B74583", "User", 9],
  ["cat_education", "spending_categories", "education", "Education", null, "#24837B", "GraduationCap", 10],
  ["cat_travel", "spending_categories", "travel", "Travel", null, "#3171B2", "Plane", 11],
  ["cat_gifts", "spending_categories", "gifts", "Gifts & Donations", null, "#AF3029", "Gift", 12],
  ["cat_fees", "spending_categories", "fees", "Fees & Charges", null, "#9C998E", "CreditCard", 13],
  ["cat_other_expense", "spending_categories", "other_expense", "Other Expenses", null, "#B6B2A4", "MoreHorizontal", 99],
  ["cat_food_restaurants", "spending_categories", "food_restaurants", "Restaurants", "cat_food", "#B89A4C", "UtensilsCrossed", 1],
  ["cat_food_coffee", "spending_categories", "food_coffee", "Coffee Shops", "cat_food", "#B89A4C", "Coffee", 2],
  ["cat_housing_rent", "spending_categories", "housing_rent", "Rent/Mortgage", "cat_housing", "#A35742", "Home", 1],
  ["cat_transport_gas", "spending_categories", "transport_gas", "Gas & Fuel", "cat_transport", "#7B96C9", "Fuel", 1],
  ["cat_shopping_clothing", "spending_categories", "shopping_clothing", "Clothing", "cat_shopping", "#8E7CB3", "Shirt", 1],
  ["cat_entertainment_streaming", "spending_categories", "entertainment_streaming", "Streaming Services", "cat_entertainment", "#B0552E", "Tv", 1],
  ["cat_health_medical", "spending_categories", "health_medical", "Medical", "cat_health", "#6B8E54", "Stethoscope", 1],
  ["cat_bills_phone", "spending_categories", "bills_phone", "Phone", "cat_bills", "#4F6B92", "Smartphone", 1],
  ["cat_salary", "income_sources", "salary", "Salary", null, "#5A7A3E", "Briefcase", 1],
  ["cat_dividends", "income_sources", "dividends", "Dividends", null, "#5A7A3E", "ChartNoAxesCombined", 2],
  ["cat_interest", "income_sources", "interest", "Interest", null, "#5A7A3E", "Landmark", 3],
  ["cat_other_income", "income_sources", "other_income", "Other Income", null, "#9C998E", "CircleDollarSign", 99],
  ["cat_savings", "savings_categories", "savings", "Savings", null, "#6B8E54", "PiggyBank", 1],
];

function json(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json; charset=utf-8", "cache-control": "no-store" },
  });
}

function taxonomyView(row) {
  return {
    id: row.id,
    name: row.name,
    color: row.color,
    description: row.description ?? null,
    isSystem: Boolean(row.is_default),
    isSingleSelect: Boolean(row.is_single_select),
    sortOrder: row.sort_order,
    createdAt: row.created_at,
    updatedAt: row.updated_at ?? row.created_at,
    scope: row.scope,
  };
}

function categoryView(row) {
  return {
    id: row.id,
    taxonomyId: row.taxonomy_id,
    parentId: row.parent_id ?? null,
    name: row.name,
    key: row.category_key,
    color: row.color,
    description: row.description ?? null,
    icon: row.icon ?? null,
    sortOrder: row.position,
    createdAt: row.created_at ?? null,
    updatedAt: row.updated_at ?? null,
  };
}

export async function ensureDefaultTaxonomies(ownerId, env) {
  const existing = await env.DB.prepare("SELECT id FROM taxonomies WHERE owner_id = ? LIMIT 1").bind(ownerId).first();
  if (existing) return;
  const statements = TAXONOMY_SEEDS.map((taxonomy) =>
    env.DB.prepare(
      "INSERT OR IGNORE INTO taxonomies (owner_id, id, name, color, description, scope, is_single_select, sort_order, is_default) VALUES (?, ?, ?, ?, ?, ?, 1, ?, 1)",
    ).bind(ownerId, taxonomy.id, taxonomy.name, taxonomy.color, taxonomy.description, taxonomy.scope, taxonomy.sortOrder),
  );
  for (let offset = 0; offset < CATEGORY_SEEDS.length; offset += 10) {
    const chunk = CATEGORY_SEEDS.slice(offset, offset + 10);
    const values = chunk.map(() => "(?, ?, ?, ?, ?, ?, ?, ?, ?, 1)").join(",");
    statements.push(
      env.DB.prepare(
        `INSERT OR IGNORE INTO categories (owner_id, id, taxonomy_id, category_key, name, parent_id, color, icon, position, is_active) VALUES ${values}`,
      ).bind(...chunk.flatMap(([id, taxonomyId, key, name, parentId, color, icon, position]) => [ownerId, id, taxonomyId, key, name, parentId, color, icon, position])),
    );
  }
  await env.DB.batch(statements);
}

export async function getActivityTaxonomies(ownerId, env) {
  await ensureDefaultTaxonomies(ownerId, env);
  const result = await env.DB.prepare(
    "SELECT id, name, color, description, scope, is_single_select, sort_order, is_default, created_at FROM taxonomies WHERE owner_id = ? AND scope = 'activity' ORDER BY sort_order, name",
  ).bind(ownerId).all();
  return result.results.map(taxonomyView);
}

export async function getActivityCategories(ownerId, env) {
  await ensureDefaultTaxonomies(ownerId, env);
  const result = await env.DB.prepare(
    "SELECT c.id, c.taxonomy_id, c.category_key, c.name, c.parent_id, c.color, c.description, c.icon, c.position, c.created_at, c.updated_at FROM categories c JOIN taxonomies t ON t.owner_id = c.owner_id AND t.id = c.taxonomy_id WHERE c.owner_id = ? AND t.scope = 'activity' AND c.is_active = 1 ORDER BY t.sort_order, c.position, c.name",
  ).bind(ownerId).all();
  return result.results.map(categoryView);
}

export async function handleTaxonomyRoute(request, route, ownerId, env) {
  if (!route.startsWith("/taxonomies")) return null;
  if (request.method !== "GET") return json({ message: "Only reading taxonomies is available in the Sites port." }, 501);
  const url = new URL(request.url);
  if (route === "/taxonomies") {
    const all = await getActivityTaxonomies(ownerId, env);
    return json(all.filter((taxonomy) => !url.searchParams.get("scope") || taxonomy.scope === url.searchParams.get("scope")));
  }
  const id = decodeURIComponent(route.slice("/taxonomies/".length).split("/")[0]);
  const taxonomy = (await getActivityTaxonomies(ownerId, env)).find((item) => item.id === id);
  if (!taxonomy) return json(null);
  const categories = (await getActivityCategories(ownerId, env)).filter((category) => category.taxonomyId === id);
  return json({ taxonomy, categories });
}

export { categoryView, taxonomyView };
