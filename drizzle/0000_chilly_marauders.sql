CREATE TABLE `accounts` (
	`id` text PRIMARY KEY NOT NULL,
	`owner_id` text NOT NULL,
	`name` text NOT NULL,
	`account_type` text NOT NULL,
	`currency` text NOT NULL,
	`group_name` text,
	`tracking_mode` text DEFAULT 'NOT_SET' NOT NULL,
	`meta` text,
	`is_default` integer DEFAULT 0 NOT NULL,
	`is_active` integer DEFAULT 1 NOT NULL,
	`is_archived` integer DEFAULT 0 NOT NULL,
	`created_at` text DEFAULT CURRENT_TIMESTAMP NOT NULL,
	`updated_at` text DEFAULT CURRENT_TIMESTAMP NOT NULL
);
--> statement-breakpoint
CREATE INDEX `accounts_owner_idx` ON `accounts` (`owner_id`);--> statement-breakpoint
CREATE TABLE `activity_category_assignments` (
	`id` text PRIMARY KEY NOT NULL,
	`owner_id` text NOT NULL,
	`activity_id` text NOT NULL,
	`taxonomy_id` text NOT NULL,
	`category_id` text NOT NULL,
	`created_at` text DEFAULT CURRENT_TIMESTAMP NOT NULL
);
--> statement-breakpoint
CREATE UNIQUE INDEX `activity_category_owner_uq` ON `activity_category_assignments` (`owner_id`,`activity_id`,`taxonomy_id`);--> statement-breakpoint
CREATE INDEX `activity_category_owner_activity_idx` ON `activity_category_assignments` (`owner_id`,`activity_id`);--> statement-breakpoint
CREATE TABLE `activity_records` (
	`id` text PRIMARY KEY NOT NULL,
	`owner_id` text NOT NULL,
	`account_id` text NOT NULL,
	`activity_date` text NOT NULL,
	`asset_id` text,
	`symbol` text,
	`activity_type` text NOT NULL,
	`quantity` text,
	`unit_price` text,
	`currency` text,
	`fee` text,
	`tax` text,
	`amount` text,
	`idempotency_key` text,
	`payload_json` text NOT NULL,
	`import_run_id` text,
	`created_at` text NOT NULL
);
--> statement-breakpoint
CREATE INDEX `activity_owner_date_idx` ON `activity_records` (`owner_id`,`activity_date`);--> statement-breakpoint
CREATE INDEX `activity_owner_account_idx` ON `activity_records` (`owner_id`,`account_id`);--> statement-breakpoint
CREATE UNIQUE INDEX `activity_owner_idempotency_uq` ON `activity_records` (`owner_id`,`idempotency_key`);--> statement-breakpoint
CREATE TABLE `asset_quotes` (
	`id` text PRIMARY KEY NOT NULL,
	`owner_id` text NOT NULL,
	`asset_id` text,
	`symbol` text,
	`quote_date` text NOT NULL,
	`price` text NOT NULL,
	`currency` text NOT NULL
);
--> statement-breakpoint
CREATE INDEX `quotes_owner_date_idx` ON `asset_quotes` (`owner_id`,`quote_date`);--> statement-breakpoint
CREATE INDEX `quotes_owner_asset_idx` ON `asset_quotes` (`owner_id`,`asset_id`);--> statement-breakpoint
CREATE TABLE `categories` (
	`id` text NOT NULL,
	`owner_id` text NOT NULL,
	`taxonomy_id` text NOT NULL,
	`category_key` text NOT NULL,
	`name` text NOT NULL,
	`parent_id` text,
	`color` text DEFAULT '#B89A4C' NOT NULL,
	`description` text,
	`icon` text,
	`position` integer DEFAULT 0 NOT NULL,
	`is_active` integer DEFAULT 1 NOT NULL,
	`created_at` text DEFAULT CURRENT_TIMESTAMP NOT NULL,
	`updated_at` text DEFAULT CURRENT_TIMESTAMP NOT NULL,
	PRIMARY KEY(`owner_id`, `id`)
);
--> statement-breakpoint
CREATE UNIQUE INDEX `categories_owner_taxonomy_key_uq` ON `categories` (`owner_id`,`taxonomy_id`,`category_key`);--> statement-breakpoint
CREATE INDEX `categories_owner_taxonomy_idx` ON `categories` (`owner_id`,`taxonomy_id`);--> statement-breakpoint
CREATE TABLE `categorization_rules` (
	`id` text PRIMARY KEY NOT NULL,
	`owner_id` text NOT NULL,
	`name` text NOT NULL,
	`pattern` text NOT NULL,
	`match_type` text DEFAULT 'contains' NOT NULL,
	`taxonomy_id` text NOT NULL,
	`category_id` text NOT NULL,
	`activity_type` text,
	`account_id` text,
	`is_global` integer DEFAULT 0 NOT NULL,
	`is_enabled` integer DEFAULT 1 NOT NULL,
	`created_at` text DEFAULT CURRENT_TIMESTAMP NOT NULL
);
--> statement-breakpoint
CREATE INDEX `rules_owner_enabled_idx` ON `categorization_rules` (`owner_id`,`is_enabled`);--> statement-breakpoint
CREATE TABLE `mcp_audit_logs` (
	`id` text PRIMARY KEY NOT NULL,
	`owner_id` text NOT NULL,
	`action` text NOT NULL,
	`result_count` integer DEFAULT 0 NOT NULL,
	`created_at` text DEFAULT CURRENT_TIMESTAMP NOT NULL
);
--> statement-breakpoint
CREATE INDEX `mcp_audit_owner_time_idx` ON `mcp_audit_logs` (`owner_id`,`created_at`);--> statement-breakpoint
CREATE TABLE `taxonomies` (
	`id` text NOT NULL,
	`owner_id` text NOT NULL,
	`name` text NOT NULL,
	`color` text DEFAULT '#B89A4C' NOT NULL,
	`description` text,
	`scope` text DEFAULT 'activity' NOT NULL,
	`is_single_select` integer DEFAULT 1 NOT NULL,
	`sort_order` integer DEFAULT 0 NOT NULL,
	`is_default` integer DEFAULT 0 NOT NULL,
	`created_at` text DEFAULT CURRENT_TIMESTAMP NOT NULL,
	PRIMARY KEY(`owner_id`, `id`)
);
--> statement-breakpoint
CREATE INDEX `taxonomies_owner_scope_idx` ON `taxonomies` (`owner_id`,`scope`);--> statement-breakpoint
CREATE TABLE `user_settings` (
	`owner_id` text PRIMARY KEY NOT NULL,
	`settings_json` text NOT NULL,
	`updated_at` text DEFAULT CURRENT_TIMESTAMP NOT NULL
);
