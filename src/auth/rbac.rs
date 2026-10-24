use std::collections::HashMap;

use sqlx::SqlitePool;
use uuid::Uuid;

pub(super) async fn seed_rbac(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let privileges = [
        ("User.Allocate", "Create or delete a user"),
        ("User.Audit", "View a user"),
        ("User.Modify", "Update a user"),
        ("Group.Allocate", "Create or delete a group"),
        ("Group.Audit", "View a group"),
        ("Group.Modify", "Update a group"),
        ("Role.Allocate", "Create or delete a role"),
        ("Role.Audit", "View a role"),
        ("Role.Modify", "Update a role"),
        ("ACE.Allocate", "Create or delete an ACL entry"),
        ("ACE.Audit", "View an ACL entry"),
        ("ACE.Modify", "Update an ACL entry"),
        ("Pool.Allocate", "Create or delete a resource pool"),
        ("Pool.Audit", "View a resource pool"),
        ("Pool.Modify", "Update a resource pool"),
        ("Template.Allocate", "Create or delete a template"),
        ("Template.Audit", "View a template"),
        ("Template.Modify", "Update a template"),
        ("Project.Allocate", "Create or delete a project"),
        ("Project.Audit", "View a project"),
        ("Project.Modify", "Update a project"),
        ("Snapshot.Allocate", "Create or delete project snapshots"),
        ("Snapshot.Restore", "Restore a snapshot"),
        ("Snapshot.Audit", "View a snapshot"),
        ("Node.Allocate", "Create or delete a node"),
        ("Node.Audit", "View a node"),
        ("Node.Modify", "Update a node"),
        ("Node.Console", "Console access to a node"),
        ("Node.PowerMgmt", "Power management for a node"),
        ("Link.Allocate", "Create or delete a link"),
        ("Link.Audit", "View a link"),
        ("Link.Modify", "Update a link"),
        ("Link.Capture", "Capture packets on a link"),
        ("Drawing.Allocate", "Create or delete a drawing"),
        ("Drawing.Audit", "View a drawing"),
        ("Drawing.Modify", "Update a drawing"),
        ("Symbol.Allocate", "Create or delete a symbol"),
        ("Symbol.Audit", "View a symbol"),
        ("Image.Allocate", "Create or delete an image"),
        ("Image.Audit", "View an image"),
        ("Compute.Allocate", "Create or delete a compute"),
        ("Compute.Modify", "Update a compute"),
        ("Compute.Audit", "View a compute"),
        ("Appliance.Allocate", "Install an appliance"),
        ("Appliance.Audit", "View an appliance"),
        ("Server.Audit", "View server settings"),
        ("Server.Modify", "Update server settings"),
    ];
    for (name, description) in privileges {
        sqlx::query("INSERT OR IGNORE INTO auth_privileges (privilege_id, name, description) VALUES (?, ?, ?)")
            .bind(Uuid::new_v4().to_string()).bind(name).bind(description).execute(pool).await?;
    }
    let roles = [
        ("Administrator", "Administrator role"),
        ("User", "Regular user role"),
        ("Auditor", "Read-only access"),
        ("Template manager", "Template management"),
        ("User manager", "User and group management"),
        ("ACL manager", "Role and ACL management"),
        ("No Access", "No access"),
    ];
    let mut role_ids = HashMap::new();
    for (name, description) in roles {
        sqlx::query("INSERT OR IGNORE INTO auth_roles (role_id, name, description, is_builtin) VALUES (?, ?, ?, 1)")
            .bind(Uuid::new_v4().to_string()).bind(name).bind(description).execute(pool).await?;
        let id: String = sqlx::query_scalar("SELECT role_id FROM auth_roles WHERE name = ?")
            .bind(name)
            .fetch_one(pool)
            .await?;
        role_ids.insert(name, id);
    }
    let all: Vec<String> = sqlx::query_scalar("SELECT privilege_id FROM auth_privileges")
        .fetch_all(pool)
        .await?;
    for privilege_id in all {
        sqlx::query(
            "INSERT OR IGNORE INTO auth_role_privileges (role_id, privilege_id) VALUES (?, ?)",
        )
        .bind(&role_ids["Administrator"])
        .bind(privilege_id)
        .execute(pool)
        .await?;
    }
    let defaults: [(&str, &[&str]); 5] = [
        (
            "User",
            &[
                "Project.Allocate",
                "Project.Audit",
                "Project.Modify",
                "Snapshot.Allocate",
                "Snapshot.Audit",
                "Snapshot.Restore",
                "Node.Allocate",
                "Node.Audit",
                "Node.Modify",
                "Node.Console",
                "Node.PowerMgmt",
                "Link.Allocate",
                "Link.Audit",
                "Link.Modify",
                "Link.Capture",
                "Drawing.Allocate",
                "Drawing.Audit",
                "Drawing.Modify",
                "Template.Audit",
                "Symbol.Audit",
                "Image.Audit",
                "Compute.Audit",
                "Appliance.Allocate",
                "Appliance.Audit",
            ],
        ),
        (
            "Auditor",
            &[
                "Project.Audit",
                "Snapshot.Audit",
                "Node.Audit",
                "Link.Audit",
                "Drawing.Audit",
                "Template.Audit",
                "Symbol.Audit",
                "Image.Audit",
                "Compute.Audit",
                "Appliance.Audit",
            ],
        ),
        (
            "Template manager",
            &[
                "Template.Allocate",
                "Template.Audit",
                "Template.Modify",
                "Symbol.Allocate",
                "Symbol.Audit",
                "Image.Allocate",
                "Image.Audit",
                "Appliance.Allocate",
                "Appliance.Audit",
            ],
        ),
        (
            "User manager",
            &[
                "User.Allocate",
                "User.Audit",
                "User.Modify",
                "Group.Allocate",
                "Group.Audit",
                "Group.Modify",
            ],
        ),
        (
            "ACL manager",
            &[
                "Role.Allocate",
                "Role.Audit",
                "Role.Modify",
                "ACE.Allocate",
                "ACE.Audit",
                "ACE.Modify",
            ],
        ),
    ];
    for (role_name, names) in defaults {
        for name in names {
            sqlx::query("INSERT OR IGNORE INTO auth_role_privileges (role_id, privilege_id) SELECT ?, privilege_id FROM auth_privileges WHERE name = ?")
                .bind(&role_ids[role_name]).bind(name).execute(pool).await?;
        }
    }
    for group in ["Administrators", "Users"] {
        sqlx::query(
            "INSERT OR IGNORE INTO auth_groups (user_group_id, name, is_builtin) VALUES (?, ?, 1)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(group)
        .execute(pool)
        .await?;
    }
    Ok(())
}
