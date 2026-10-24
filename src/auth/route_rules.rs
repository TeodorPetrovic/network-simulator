#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RouteClass {
    Public,
    PrincipalScoped,
    Deny,
    Privilege(&'static str),
}

#[derive(Clone, Copy)]
pub(super) struct RouteRule {
    pub(super) method: &'static str,
    pub(super) path: &'static str,
    pub(super) class: RouteClass,
}

#[path = "route_rules_table.rs"]
mod route_rules_table;
pub(super) use route_rules_table::ROUTE_RULES;
