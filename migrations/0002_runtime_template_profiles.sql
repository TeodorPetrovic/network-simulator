CREATE TABLE runtime_template_profiles (
    template_id TEXT PRIMARY KEY NOT NULL,
    template_type TEXT NOT NULL CHECK (template_type IN ('qemu', 'docker', 'dynamips', 'vpcs')),
    template_json TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
