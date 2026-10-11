//! Shared by the contract tests that pin the size of a page (#365).

use serde_json::Value;

/// The characters of the page of fifty that `response` makes when the items of
/// its `data` (and, for a search, its hits) are repeated to fifty, as compact
/// JSON on one line.
///
/// This is the method behind every figure of `docs/contracts/cli-v1.md`
/// ("Views of a read", "Size"): take the committed example of an answer in each
/// view, repeat its items (and its hits) in order until there are fifty, and
/// count the characters. The examples' segments are short, so a real page is
/// larger in both views and the brief share of it is higher.
///
/// # Errors
///
/// Returns the serialization error when the page cannot be written as JSON.
pub fn page_of_fifty(response: &Value) -> Result<usize, serde_json::Error> {
    let mut page = response.clone();
    for member in ["items", "hits"] {
        if let Some(Value::Array(list)) = page["data"].get_mut(member) {
            let repeated: Vec<Value> = list.iter().cloned().cycle().take(50).collect();
            if !list.is_empty() {
                *list = repeated;
            }
        }
    }
    Ok(serde_json::to_string(&page)?.len())
}
