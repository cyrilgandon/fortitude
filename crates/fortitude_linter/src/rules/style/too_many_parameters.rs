use crate::ast::FortitudeNode;
use crate::settings::CheckSettings;
use crate::symbol_table::SymbolTables;
use crate::{AstRule, FromAstNode};
use ruff_diagnostics::{Diagnostic, Violation};
use ruff_macros::{ViolationMetadata, derive_message_formats};
use ruff_source_file::SourceFile;
use tree_sitter::Node;

/// ## What it does
/// Checks for functions or subroutines with more parameters than a configurable threshold (default: 5).
///
/// ## Why is this bad?
/// Too many parameters make code harder to maintain and test.
///
/// ## Example
/// ```f90
/// subroutine foo(a, b, c, d, e, f)
/// end subroutine foo
/// ```
#[derive(ViolationMetadata)]
pub(crate) struct TooManyParameters {
    pub name: String,
    pub count: usize,
    pub threshold: usize,
}

impl Violation for TooManyParameters {
    #[derive_message_formats]
    fn message(&self) -> String {
        let Self { name, count, threshold } = self;
        format!("Function/subroutine '{name}' has {count} parameters (threshold: {threshold})")
    }
}

impl AstRule for TooManyParameters {
    fn check<'a>(
        settings: &CheckSettings,
        node: &'a Node,
        src: &'a SourceFile,
        _symbol_table: &SymbolTables,
    ) -> Option<Vec<Diagnostic>> {
        let header = node.named_child(0)?;
        let name = header.child_by_field_name("name")?.to_text(src.source_text()).unwrap_or("").to_string();
        let params_node = header.child_by_field_name("parameters");
        let count = if let Some(params) = params_node {
            params.named_children(&mut params.walk()).count()
        } else {
            0
        };
        let threshold = settings.too_many_parameters.max_parameters;
        if count > threshold {
            return some_vec![Diagnostic::from_node(
                TooManyParameters {
                    name,
                    count,
                    threshold,
                },
                &header,
            )];
        }
        None
    }

    fn entrypoints() -> Vec<&'static str> {
        vec!["function", "subroutine"]
    }
}

pub mod settings {
    use crate::display_settings;
    use ruff_macros::CacheKey;
    use std::fmt::Display;

    #[derive(Debug, Clone, CacheKey)]
    pub struct Settings {
        pub max_parameters: usize,
    }

    impl Default for Settings {
        fn default() -> Self {
            Self { max_parameters: 5 }
        }
    }

    impl Display for Settings {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            display_settings! {
                formatter = f,
                namespace = "check.too-many-parameters",
                fields = [self.max_parameters]
            }
            Ok(())
        }
    }
}
