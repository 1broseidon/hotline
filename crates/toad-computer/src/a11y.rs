use atspi::CoordType;
use atspi::proxy::accessible::{AccessibleProxy, ObjectRefExt};
use atspi::proxy::bus::BusProxy;
use atspi::proxy::proxy_ext::ProxyExt;

use crate::x11::Window;

const MAX_DEPTH: usize = 15;
const MAX_NODES: usize = 400;

#[derive(Debug)]
struct Node {
    depth: usize,
    role: String,
    name: String,
    bounds: [i32; 4],
}

pub async fn tree(windows: &[Window]) -> String {
    let queried = query().await.unwrap_or_default();
    let mut output = String::new();
    for window in windows {
        output.push_str(&format!(
            "[{} {} {},{} {}x{}]\n",
            window.id,
            truncate(&window.title, 60),
            window.bounds[0],
            window.bounds[1],
            window.bounds[2],
            window.bounds[3]
        ));
        if let Some((_, nodes)) = queried
            .iter()
            .find(|(title, _)| titles_match(title, &window.title))
        {
            for node in nodes {
                output.push_str(&"  ".repeat(node.depth + 1));
                output.push_str(&format!(
                    "[{}] {} {},{} {}x{}\n",
                    node.role,
                    truncate(&node.name, 200),
                    node.bounds[0],
                    node.bounds[1],
                    node.bounds[2],
                    node.bounds[3]
                ));
            }
        }
        output.push('\n');
    }
    if output.is_empty() {
        output.push_str("[desktop no windows]\n");
    }
    output
}

async fn query() -> Result<Vec<(String, Vec<Node>)>, String> {
    let session = atspi::zbus::Connection::session()
        .await
        .map_err(|error| format!("session D-Bus: {error}"))?;
    let address = BusProxy::new(&session)
        .await
        .map_err(|error| error.to_string())?
        .get_address()
        .await
        .map_err(|error| error.to_string())?;
    let connection = atspi::zbus::connection::Builder::address(address.as_str())
        .map_err(|error| error.to_string())?
        .build()
        .await
        .map_err(|error| error.to_string())?;
    let registry = AccessibleProxy::builder(&connection)
        .destination("org.a11y.atspi.Registry")
        .map_err(|error| error.to_string())?
        .path("/org/a11y/atspi/accessible/root")
        .map_err(|error| error.to_string())?
        .build()
        .await
        .map_err(|error| error.to_string())?;

    let mut result = Vec::new();
    for application in registry.get_children().await.unwrap_or_default() {
        let Ok(application) = application.as_accessible_proxy(&connection).await else {
            continue;
        };
        for window_ref in application.get_children().await.unwrap_or_default() {
            let Ok(window) = window_ref.as_accessible_proxy(&connection).await else {
                continue;
            };
            let title = window.name().await.unwrap_or_default();
            if title.is_empty() {
                continue;
            }
            let children = window.get_children().await.unwrap_or_default();
            let mut stack: Vec<_> = children
                .into_iter()
                .rev()
                .map(|child| (child, 0_usize))
                .collect();
            let mut nodes = Vec::new();
            while let Some((reference, depth)) = stack.pop() {
                if nodes.len() >= MAX_NODES || depth >= MAX_DEPTH {
                    continue;
                }
                let Ok(proxy) = reference.as_accessible_proxy(&connection).await else {
                    continue;
                };
                let name = proxy.name().await.unwrap_or_default();
                let role = proxy
                    .get_role()
                    .await
                    .map(|role| role.name().to_owned())
                    .unwrap_or_else(|_| "unknown".to_owned());
                let bounds = match proxy.proxies().await {
                    Ok(proxies) => match proxies.component().await {
                        Ok(component) => component
                            .get_extents(CoordType::Screen)
                            .await
                            .map(|(x, y, width, height)| [x, y, width, height])
                            .unwrap_or([0; 4]),
                        Err(_) => [0; 4],
                    },
                    Err(_) => [0; 4],
                };
                if !name.is_empty() || bounds[2] > 0 || bounds[3] > 0 {
                    nodes.push(Node {
                        depth,
                        role,
                        name,
                        bounds,
                    });
                }
                let mut children = proxy.get_children().await.unwrap_or_default();
                children.reverse();
                stack.extend(children.into_iter().map(|child| (child, depth + 1)));
            }
            result.push((title, nodes));
        }
    }
    Ok(result)
}

fn titles_match(left: &str, right: &str) -> bool {
    let left = left.to_lowercase();
    let right = right.to_lowercase();
    left == right || left.contains(&right) || right.contains(&left)
}

fn truncate(value: &str, max: usize) -> String {
    let mut chars = value.chars();
    let prefix: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        format!(
            "{}...",
            prefix
                .chars()
                .take(max.saturating_sub(3))
                .collect::<String>()
        )
    } else {
        prefix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_titles_ignore_case_and_decorations() {
        assert!(titles_match("Proof", "proof — Chromium"));
        assert!(!titles_match("Proof", "Editor"));
    }
}
