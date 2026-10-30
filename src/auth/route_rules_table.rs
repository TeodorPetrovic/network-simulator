use super::{RouteClass, RouteRule};

macro_rules! rules {
    ($(($method:literal, $path:literal, $class:expr)),* $(,)?) => {
        &[$(RouteRule { method: $method, path: $path, class: $class }),*]
    };
}

pub const ROUTE_RULES: &[RouteRule] = rules![
    ("POST", "/access/users/login", RouteClass::Public),
    ("POST", "/access/users/authenticate", RouteClass::Public),
    ("POST", "/access/users/refresh", RouteClass::Public),
    ("GET", "/access/settings/status", RouteClass::Public),
    ("POST", "/access/users/logout", RouteClass::PrincipalScoped),
    ("GET", "/access/users/me", RouteClass::PrincipalScoped),
    ("PUT", "/access/users/me", RouteClass::PrincipalScoped),
    ("POST", "/access/ws-ticket", RouteClass::PrincipalScoped),
    ("GET", "/access/api-keys", RouteClass::PrincipalScoped),
    ("POST", "/access/api-keys", RouteClass::PrincipalScoped),
    (
        "POST",
        "/access/api-keys/{key_id}/revoke",
        RouteClass::PrincipalScoped
    ),
    (
        "POST",
        "/access/api-keys/{key_id}/restore",
        RouteClass::PrincipalScoped
    ),
    (
        "DELETE",
        "/access/api-keys/{key_id}",
        RouteClass::PrincipalScoped
    ),
    ("GET", "/access/users", RouteClass::Privilege("User.Audit")),
    (
        "POST",
        "/access/users",
        RouteClass::Privilege("User.Allocate")
    ),
    (
        "GET",
        "/access/users/{user_id}",
        RouteClass::Privilege("User.Audit")
    ),
    (
        "PUT",
        "/access/users/{user_id}",
        RouteClass::Privilege("User.Modify")
    ),
    (
        "DELETE",
        "/access/users/{user_id}",
        RouteClass::Privilege("User.Allocate")
    ),
    (
        "GET",
        "/access/users/{user_id}/groups",
        RouteClass::Privilege("User.Audit")
    ),
    (
        "GET",
        "/access/groups",
        RouteClass::Privilege("Group.Audit")
    ),
    (
        "POST",
        "/access/groups",
        RouteClass::Privilege("Group.Allocate")
    ),
    (
        "GET",
        "/access/groups/{group_id}",
        RouteClass::Privilege("Group.Audit")
    ),
    (
        "PUT",
        "/access/groups/{group_id}",
        RouteClass::Privilege("Group.Modify")
    ),
    (
        "DELETE",
        "/access/groups/{group_id}",
        RouteClass::Privilege("Group.Allocate")
    ),
    (
        "GET",
        "/access/groups/{group_id}/members",
        RouteClass::Privilege("Group.Audit")
    ),
    (
        "PUT",
        "/access/groups/{group_id}/members/{user_id}",
        RouteClass::Privilege("Group.Modify")
    ),
    (
        "DELETE",
        "/access/groups/{group_id}/members/{user_id}",
        RouteClass::Privilege("Group.Modify")
    ),
    ("GET", "/access/roles", RouteClass::Privilege("Role.Audit")),
    (
        "POST",
        "/access/roles",
        RouteClass::Privilege("Role.Allocate")
    ),
    (
        "GET",
        "/access/roles/{role_id}",
        RouteClass::Privilege("Role.Audit")
    ),
    (
        "PUT",
        "/access/roles/{role_id}",
        RouteClass::Privilege("Role.Modify")
    ),
    (
        "DELETE",
        "/access/roles/{role_id}",
        RouteClass::Privilege("Role.Allocate")
    ),
    (
        "GET",
        "/access/roles/{role_id}/privileges",
        RouteClass::Privilege("Role.Audit")
    ),
    (
        "PUT",
        "/access/roles/{role_id}/privileges/{privilege_id}",
        RouteClass::Privilege("Role.Modify")
    ),
    (
        "DELETE",
        "/access/roles/{role_id}/privileges/{privilege_id}",
        RouteClass::Privilege("Role.Modify")
    ),
    (
        "GET",
        "/access/privileges",
        RouteClass::Privilege("Role.Audit")
    ),
    ("GET", "/access/acl", RouteClass::Privilege("ACE.Audit")),
    ("POST", "/access/acl", RouteClass::Privilege("ACE.Allocate")),
    (
        "GET",
        "/access/acl/endpoints",
        RouteClass::Privilege("ACE.Audit")
    ),
    (
        "GET",
        "/access/acl/{ace_id}",
        RouteClass::Privilege("ACE.Audit")
    ),
    (
        "PUT",
        "/access/acl/{ace_id}",
        RouteClass::Privilege("ACE.Modify")
    ),
    (
        "DELETE",
        "/access/acl/{ace_id}",
        RouteClass::Privilege("ACE.Allocate")
    ),
    (
        "GET",
        "/access/settings",
        RouteClass::Privilege("Server.Audit")
    ),
    (
        "PUT",
        "/access/settings",
        RouteClass::Privilege("Server.Modify")
    ),
    ("GET", "/pools", RouteClass::Privilege("Pool.Audit")),
    ("POST", "/pools", RouteClass::Privilege("Pool.Allocate")),
    (
        "GET",
        "/pools/{pool_id}",
        RouteClass::Privilege("Pool.Audit")
    ),
    (
        "PUT",
        "/pools/{pool_id}",
        RouteClass::Privilege("Pool.Modify")
    ),
    (
        "DELETE",
        "/pools/{pool_id}",
        RouteClass::Privilege("Pool.Allocate")
    ),
    (
        "GET",
        "/pools/{pool_id}/resources",
        RouteClass::Privilege("Pool.Audit")
    ),
    (
        "PUT",
        "/pools/{pool_id}/resources/{resource_id}",
        RouteClass::Privilege("Pool.Modify")
    ),
    (
        "DELETE",
        "/pools/{pool_id}/resources/{resource_id}",
        RouteClass::Privilege("Pool.Modify")
    ),
    ("GET", "/version", RouteClass::Privilege("Server.Audit")),
    ("POST", "/version", RouteClass::Privilege("Project.Modify")),
    ("GET", "/runtimes", RouteClass::Privilege("Server.Audit")),
    (
        "GET",
        "/resources/monitor",
        RouteClass::Privilege("Server.Audit")
    ),
    (
        "POST",
        "/docker/images/pull",
        RouteClass::Privilege("Compute.Modify")
    ),
    (
        "GET",
        "/marketplace/ishare2/images",
        RouteClass::Privilege("Server.Audit")
    ),
    (
        "POST",
        "/marketplace/ishare2/images/{image_type}/{image_id}/download",
        RouteClass::Privilege("Image.Allocate")
    ),
    (
        "GET",
        "/marketplace/gns3/appliances",
        RouteClass::Privilege("Appliance.Audit")
    ),
    (
        "GET",
        "/marketplace/gns3/appliances/{file_name}",
        RouteClass::Privilege("Appliance.Audit")
    ),
    (
        "POST",
        "/marketplace/gns3/appliances/{file_name}/import",
        RouteClass::Privilege("Appliance.Allocate")
    ),
    (
        "GET",
        "/marketplace/gns3/imported",
        RouteClass::Privilege("Appliance.Audit")
    ),
    (
        "GET",
        "/marketplace/gns3/manager",
        RouteClass::Privilege("Appliance.Audit")
    ),
    (
        "POST",
        "/marketplace/gns3/custom",
        RouteClass::Privilege("Template.Allocate")
    ),
    (
        "PUT",
        "/marketplace/gns3/appliances/{file_name}/bindings",
        RouteClass::Privilege("Appliance.Allocate")
    ),
    (
        "DELETE",
        "/marketplace/gns3/appliances/{file_name}/bindings",
        RouteClass::Privilege("Appliance.Allocate")
    ),
    (
        "POST",
        "/marketplace/gns3/import/{file_name}",
        RouteClass::Privilege("Appliance.Allocate")
    ),
    ("GET", "/statistics", RouteClass::Privilege("Server.Audit")),
    (
        "GET",
        "/notifications",
        RouteClass::Privilege("Server.Audit")
    ),
    (
        "GET",
        "/notifications/ws",
        RouteClass::Privilege("Server.Audit")
    ),
    ("GET", "/settings", RouteClass::Privilege("Server.Audit")),
    ("PUT", "/settings", RouteClass::Privilege("Server.Modify")),
    ("GET", "/iou_license", RouteClass::Privilege("Server.Audit")),
    (
        "PUT",
        "/iou_license",
        RouteClass::Privilege("Server.Modify")
    ),
    ("GET", "/computes", RouteClass::Privilege("Compute.Audit")),
    ("POST", "/computes", RouteClass::Privilege("Compute.Modify")),
    (
        "GET",
        "/computes/{compute_id}",
        RouteClass::Privilege("Compute.Audit")
    ),
    ("PUT", "/computes/{compute_id}", RouteClass::Privilege("Compute.Modify")),
    ("DELETE", "/computes/{compute_id}", RouteClass::Privilege("Compute.Modify")),
    (
        "POST",
        "/computes/{compute_id}/disconnect",
        RouteClass::Privilege("Compute.Modify")
    ),
    ("GET", "/templates", RouteClass::Privilege("Template.Audit")),
    (
        "POST",
        "/templates",
        RouteClass::Privilege("Template.Allocate")
    ),
    (
        "GET",
        "/templates/{template_id}",
        RouteClass::Privilege("Template.Audit")
    ),
    (
        "PUT",
        "/templates/{template_id}",
        RouteClass::Privilege("Template.Modify")
    ),
    (
        "DELETE",
        "/templates/{template_id}",
        RouteClass::Privilege("Template.Allocate")
    ),
    (
        "POST",
        "/templates/{template_id}/duplicate",
        RouteClass::Privilege("Template.Allocate")
    ),
    (
        "POST",
        "/projects/load",
        RouteClass::Privilege("Project.Allocate")
    ),
];
