use super::route_rules::{RouteClass, ROUTE_RULES};

pub(super) fn classify_route(method: &str, path: &str) -> Option<RouteClass> {
    ROUTE_RULES
        .iter()
        .find(|rule| {
            (rule.method == method || rule.method == "ANY") && route_path_matches(path, rule.path)
        })
        .map(|rule| rule.class)
}

fn route_path_matches(path: &str, pattern: &str) -> bool {
    let path_segments = path.trim_matches('/').split('/').collect::<Vec<_>>();
    let pattern_segments = pattern.trim_matches('/').split('/').collect::<Vec<_>>();
    let mut path_index = 0;
    for (pattern_index, pattern_segment) in pattern_segments.iter().enumerate() {
        if pattern_segment.starts_with("{*") && pattern_segment.ends_with('}') {
            return pattern_index + 1 == pattern_segments.len() && path_index < path_segments.len();
        }
        let Some(path_segment) = path_segments.get(path_index) else {
            return false;
        };
        if !(pattern_segment.starts_with('{') && pattern_segment.ends_with('}'))
            && pattern_segment != path_segment
        {
            return false;
        }
        path_index += 1;
    }
    path_index == path_segments.len()
}
