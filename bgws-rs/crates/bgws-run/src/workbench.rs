//! The analysis workbench (§16): one self-contained HTML page with the
//! vignettes against their bands and baselines, and sensitivity rankings.
//! The page draws its charts from the JSON written into it here.

use serde_json::Value;

const TEMPLATE: &str = include_str!("../../../web/workbench/workbench.html");

/// The page for `data`: `{title, generated, vignettes?, regression?, backlog?, sensitivity?}`.
pub fn html(data: &Value) -> String {
    // Inside a <script> block, "</" would end it early.
    let json = data.to_string().replace("</", "<\\/");
    TEMPLATE.replace("__BGWS_DATA__", &json)
}

#[cfg(test)]
mod tests {
    #[test]
    fn data_goes_in_once_and_cannot_close_the_script() {
        let page = super::html(&serde_json::json!({ "title": "</script><b>x" }));
        assert!(!page.contains("__BGWS_DATA__"));
        assert!(page.contains("<\\/script><b>x"));
        assert_eq!(page.matches("</script>").count(), 2);
    }
}
