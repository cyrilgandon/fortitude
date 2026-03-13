use crate::AstRule;
use crate::ast::FortitudeNode;
use crate::settings::CheckSettings;
use crate::symbol_table::SymbolTables;
use crate::traits::TextRanged;
use ruff_diagnostics::{AlwaysFixableViolation, Diagnostic, Edit, Fix};
use ruff_macros::{ViolationMetadata, derive_message_formats};
use ruff_source_file::{LineRanges, SourceFile};
use ruff_text_size::TextRange;
use tree_sitter::Node;

/// ## What it does
/// Checks that `use` statements are sorted alphabetically within contiguous blocks.
/// Intrinsic modules (`use, intrinsic ::`) are always placed first.
///
/// ## Why is this bad?
/// Sorted imports are easier to scan, reduce cognitive load when reviewing code,
/// and help avoid merge conflicts when multiple developers add imports to the same block.
///
/// ## Example
/// ```f90
/// ! Not recommended
/// use module_c, only: fun_c
/// use, intrinsic :: iso_fortran_env, only: int32
/// use module_a, only: fun_a
/// use module_b, only: fun_b
///
/// ! Better
/// use, intrinsic :: iso_fortran_env, only: int32
/// use module_a, only: fun_a
/// use module_b, only: fun_b
/// use module_c, only: fun_c
/// ```
///
/// Blocks of `use` statements separated by blank lines are sorted independently.
#[derive(ViolationMetadata)]
pub(crate) struct UnsortedUses {}

impl AlwaysFixableViolation for UnsortedUses {
    #[derive_message_formats]
    fn message(&self) -> String {
        "`use` statements are not sorted".to_string()
    }

    fn fix_title(&self) -> String {
        "Sort `use` statements".to_string()
    }
}

impl AstRule for UnsortedUses {
    fn check(
        _settings: &CheckSettings,
        node: &Node,
        src: &SourceFile,
        _symbol_table: &SymbolTables,
    ) -> Option<Vec<Diagnostic>> {
        let use_statements: Vec<UseStatementData> = node
            .children(&mut node.walk())
            .filter(|child| child.kind() == "use_statement")
            .map(|child| extract_use_statement_data(&child, src))
            .collect();

        if use_statements.len() <= 1 {
            return None;
        }
        // Group use statements into blocks separated by empty lines
        let blocks = group_use_statements_into_blocks(&use_statements);

        let mut diagnostics = Vec::new();

        for block in &blocks {
            if block.len() <= 1 {
                continue;
            }

            let mut sorted: Vec<&UseStatementData> = block.to_vec();
            sorted.sort_by(|a, b| compare_use_statements(a, b));

            let is_sorted = block
                .iter()
                .zip(sorted.iter())
                .all(|(orig, s)| orig.text == s.text);

            if is_sorted {
                continue;
            }

            let block_start = src
                .source_text()
                .line_start(block.first()?.text_range.start());
            let block_end = src
                .source_text()
                .full_line_end(block.last()?.text_range.end());

            let replacement = sorted.iter().map(|s| s.text.as_str()).collect::<String>();
            let edit = Edit::range_replacement(replacement, TextRange::new(block_start, block_end));
            let fix = Fix::safe_edit(edit);

            let first = block.first()?;
            let diag = Diagnostic::new(UnsortedUses {}, first.text_range).with_fix(fix);
            diagnostics.push(diag);
        }

        if diagnostics.is_empty() {
            None
        } else {
            Some(diagnostics)
        }
    }

    fn entrypoints() -> Vec<&'static str> {
        vec!["module", "submodule", "program", "subroutine", "function"]
    }
}

/// Groups indices of `use` statements into contiguous blocks.
fn group_use_statements_into_blocks<'a>(
    all_use_statements: &'a [UseStatementData],
) -> Vec<Vec<&'a UseStatementData>> {
    let mut last_row: Option<usize> = None;

    let use_statements: Vec<&UseStatementData> = all_use_statements
        .iter()
        .filter(|child| {
            let row = child.start_position_row;
            if Some(row) == last_row {
                false
            } else {
                last_row = Some(row);
                true
            }
        })
        .collect();

    if use_statements.is_empty() {
        return Vec::new();
    }
    let mut blocks: Vec<Vec<&'a UseStatementData>> = Vec::new();
    let mut current_block = vec![use_statements[0]];

    for i in 1..use_statements.len() {
        let prev = &use_statements[i - 1];
        let curr = &use_statements[i];

        if are_statements_adjacent(prev, curr) {
            current_block.push(curr);
        } else {
            blocks.push(current_block);
            current_block = vec![curr];
        }
    }

    blocks.push(current_block);
    blocks
}

/// Two use statements are considered adjacent if the second one starts
/// on the line immediately following the end of the first one.
fn are_statements_adjacent(stmt1: &UseStatementData, stmt2: &UseStatementData) -> bool {
    let line1 = stmt1.end_position_row;
    let line2 = stmt2.start_position_row;
    line2 == line1 + 1
}

struct UseStatementData {
    text_range: TextRange,
    start_position_row: usize,
    end_position_row: usize,
    text: String,
    module_name: String,
    is_intrinsic: bool,
    only_items: Vec<OnlyStatementData>,
}

#[derive(Clone)]
struct OnlyStatementData {
    /// The item name (without "as" alias), normalized to lowercase for sorting.
    name: String,
    /// The alias if present (e.g., "alias" from "fun_1 as alias").
    alias: Option<String>,
    /// The inline comment associated with this item (e.g., "!! comment").
    inline_comment: Option<String>,
}

fn extract_use_statement_data(node: &Node, src: &SourceFile) -> UseStatementData {
    let range = node.textrange();
    let text = src.source_text().full_lines_str(range).to_string();

    let module_name = node
        .module_name(src.source_text())
        // Fortran is case-insensitive, normalize to lowercase for consistent sorting
        .map(|s| s.to_lowercase())
        .unwrap_or_default();

    let is_intrinsic = node
        .children(&mut node.walk())
        .any(|child| child.to_text(src.source_text()) == Some("intrinsic"));

    let only_items = extract_only_items(&text);

    UseStatementData {
        text_range: node.textrange(),
        start_position_row: node.start_position().row,
        end_position_row: node.end_position().row,
        text,
        module_name,
        is_intrinsic,
        only_items,
    }
}

// Intrinsic modules (e.g. `use, intrinsic :: iso_fortran_env`) always come first,
// followed by regular modules sorted alphabetically by name.
fn compare_use_statements(a: &UseStatementData, b: &UseStatementData) -> std::cmp::Ordering {
    match (a.is_intrinsic, b.is_intrinsic) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.module_name.cmp(&b.module_name),
    }
}

fn extract_only_items(text: &str) -> Vec<OnlyStatementData> {
    if !text.contains("only:") {
        return Vec::new();
    }
    let after = text.split("only:").nth(1).unwrap().trim_start();
    
    // Split by newlines and process each line as a potential item
    let mut items = Vec::new();
    for line in after.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Remove trailing comma and continuation if present
        let line = line.trim_end_matches(", &").trim_end_matches(',').trim();
        if !line.is_empty() {
            items.push(extract_name_and_comment(line));
        }
    }
    
    items
}

fn extract_name_and_comment(item: &str) -> OnlyStatementData {
    // Remove continuation markers and normalize whitespace
    let item_clean = item.replace("&", "");
    let item = item_clean.trim();

    // Separate out inline comment if present
    let (raw_item, inline_comment) = if let Some(comment_pos) = item.find("!!") {
        let name_part = item[..comment_pos].trim();
        let comment_part = item[comment_pos..].trim();
        (name_part, Some(comment_part.to_string()))
    } else {
        (item, None)
    };

    // Remove any trailing commas
    let raw_item = raw_item.trim_end_matches(',').trim();

    // Parse alias if present
    let (name, alias) = if let Some(as_pos) = raw_item.to_lowercase().find(" as ") {
        // Keep original casing for alias
        let (left, right) = raw_item.split_at(as_pos);
        let alias = right[4..].trim(); // skip " as "
        (left.trim().to_lowercase(), Some(alias.to_string()))
    } else {
        (raw_item.to_lowercase(), None)
    };

    OnlyStatementData {
        name,
        alias,
        inline_comment,
    }
}

fn extract_item_name(item: &OnlyStatementData) -> String {
    // Already stored as lowercase base name for sorting
    item.name.clone()
}


#[derive(ViolationMetadata)]
pub(crate) struct UnsortedOnlys {}

impl AlwaysFixableViolation for UnsortedOnlys {
    #[derive_message_formats]
    fn message(&self) -> String {
        "Items in `only` clause are not sorted".to_string()
    }

    fn fix_title(&self) -> String {
        "Sort items in `only` clause".to_string()
    }
}

impl AstRule for UnsortedOnlys {
    fn check(
        _settings: &CheckSettings,
        node: &Node,
        src: &SourceFile,
        _symbol_table: &SymbolTables,
    ) -> Option<Vec<Diagnostic>> {
        let use_stmt = extract_use_statement_data(node, src);

        if !use_stmt.only_items.is_empty() && use_stmt.only_items.len() > 1 {
            // Sort by name
            let mut sorted_items = use_stmt.only_items.clone();
            sorted_items.sort_by(|a, b| extract_item_name(a).cmp(&extract_item_name(b)));
            
            // Check if already sorted
            let is_sorted = use_stmt.only_items.iter()
                .map(|item| extract_item_name(item))
                .collect::<Vec<_>>() == sorted_items.iter()
                .map(|item| extract_item_name(item))
                .collect::<Vec<_>>();
            
            if !is_sorted {
                // Reconstruct the only clause with sorted items
                let sorted_texts: Vec<String> = sorted_items.iter()
                    .map(|item| {
                        // Rebuild item text using parsed name + alias to ensure alias is preserved
                        let mut text = item.name.clone();
                        if let Some(alias) = &item.alias {
                            text.push_str(" as ");
                            text.push_str(alias);
                        }
                        if let Some(ref comment) = item.inline_comment {
                            text.push_str(" ");
                            text.push_str(comment);
                        }
                        text
                    })
                    .collect();
                let sorted_only = sorted_texts.join(", ");
                let before = use_stmt.text.split("only:").nth(0).unwrap();
                let replacement = format!("{}only: {}", before, sorted_only);
                let edit = Edit::range_replacement(replacement, use_stmt.text_range);
                let fix = Fix::safe_edit(edit);
                let diag = Diagnostic::new(UnsortedOnlys {}, use_stmt.text_range).with_fix(fix);
                return Some(vec![diag]);
            }
        }

        None
    }

    fn entrypoints() -> Vec<&'static str> {
        vec!["use_statement"]
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use ruff_source_file::SourceFileBuilder;
    use tree_sitter::Parser;

    use crate::rules::style::use_statement::{
        UseStatementData, OnlyStatementData, extract_use_statement_data, group_use_statements_into_blocks,
        extract_item_name,
    };

    #[test]
    fn test_group_use_statements_into_blocks() -> Result<()> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_fortran::LANGUAGE.into())
            .context("Error loading Fortran grammar")?;

        // Block 1: alpha, beta
        // blank line separator
        // Block 2: charlie, delta
        // comment separator
        // Block 3: echo (alone)
        // Block 4: only foxtrot is kept — golf is on the same line and must be ignored
        let code = {
            r#"
        program foo
          use alpha_module
          use beta_module

          use charlie_module
          use delta_module
          ! a comment acts as a separator
          use echo_module

          use foxtrot_module; use golf_module
        end program foo
    "#
        };

        let tree = parser.parse(code, None).context("Failed to parse")?;
        let src = SourceFileBuilder::new("test.f90", code).finish();

        let program_node = tree.root_node().child(0).context("Missing program node")?;
        assert_eq!(program_node.kind(), "program");

        let use_statements: Vec<UseStatementData> = program_node
            .children(&mut program_node.walk())
            .filter(|child| child.kind() == "use_statement")
            .map(|child| extract_use_statement_data(&child, &src))
            .collect();
        let blocks = group_use_statements_into_blocks(&use_statements);
        let block_names = |block: &Vec<&UseStatementData>| -> Vec<String> {
            block.iter().map(|s| s.module_name.clone()).collect()
        };

        assert_eq!(blocks.len(), 4, "expected 4 blocks");
        assert_eq!(block_names(&blocks[0]), vec!["alpha_module", "beta_module"]);
        assert_eq!(
            block_names(&blocks[1]),
            vec!["charlie_module", "delta_module"]
        );
        assert_eq!(block_names(&blocks[2]), vec!["echo_module"]);
        assert_eq!(block_names(&blocks[3]), vec!["foxtrot_module"]); // golf_module ignored: same line

        Ok(())
    }
    #[test]
    fn test_extract_use_statement_data() -> Result<()> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_fortran::LANGUAGE.into())
            .context("Error loading Fortran grammar")?;

        let code = {
            r#"
        program foo
          use iso_fortran_env, only: real64
          use, intrinsic :: iso_c_binding, only: c_int
          use My_Module
          use foxtrot_module; use golf_module
          use multiline_module, only: fun_1, &
                                      fun_2, & !! 123_comments
                                      fun_3
        end program foo
    "#
        };

        let tree = parser.parse(code, None).context("Failed to parse")?;
        let root = tree.root_node().child(0).context("Missing child")?;
        let src = SourceFileBuilder::new("test.f90", code).finish();

        let all_use_statements: Vec<UseStatementData> = root
            .children(&mut root.walk())
            .filter(|child| child.kind() == "use_statement")
            .map(|child| extract_use_statement_data(&child, &src))
            .collect();

        assert_eq!(all_use_statements.len(), 6);

        // Test regular use statement
        let regular = &all_use_statements[0];
        assert!(!regular.is_intrinsic);
        assert_eq!(regular.module_name, "iso_fortran_env");
        assert_eq!(regular.start_position_row, 2);
        assert_eq!(regular.end_position_row, 2);
        assert!(regular.text.contains("iso_fortran_env"));
        assert!(!regular.text_range.is_empty());

        // Test intrinsic use statement
        let intrinsic = &all_use_statements[1];
        assert!(intrinsic.is_intrinsic);
        assert_eq!(intrinsic.module_name, "iso_c_binding");
        assert_eq!(intrinsic.start_position_row, 3);
        assert_eq!(intrinsic.end_position_row, 3);
        assert!(intrinsic.text.contains("iso_c_binding"));
        assert!(!intrinsic.text_range.is_empty());

        // Test mixed case use statement
        let mixed_case = &all_use_statements[2];
        assert!(!mixed_case.is_intrinsic);
        assert_eq!(mixed_case.module_name, "my_module");
        assert_eq!(mixed_case.start_position_row, 4);
        assert_eq!(mixed_case.end_position_row, 4);
        assert!(mixed_case.text.contains("My_Module"));
        assert!(!mixed_case.text_range.is_empty());

        // Test foxtrot_module (first on same line)
        let foxtrot = &all_use_statements[3];
        assert!(!foxtrot.is_intrinsic);
        assert_eq!(foxtrot.module_name, "foxtrot_module");
        assert_eq!(foxtrot.start_position_row, 5);
        assert_eq!(foxtrot.end_position_row, 5);
        assert!(foxtrot.text.contains("foxtrot_module"));
        assert!(!foxtrot.text_range.is_empty());

        // Test golf_module (second on same line)
        let golf = &all_use_statements[4];
        assert!(!golf.is_intrinsic);
        assert_eq!(golf.module_name, "golf_module");
        assert_eq!(golf.start_position_row, 5);
        assert_eq!(golf.end_position_row, 5);
        assert!(golf.text.contains("golf_module"));
        assert!(!golf.text_range.is_empty());

        // Test multiline_module
        let multiline = &all_use_statements[5];
        assert!(!multiline.is_intrinsic);
        assert_eq!(multiline.module_name, "multiline_module");
        assert_eq!(multiline.start_position_row, 6);
        assert_eq!(multiline.end_position_row, 8);
        assert!(multiline.text.contains("fun_1"));
        assert!(multiline.text.contains("fun_2"));
        assert!(multiline.text.contains("fun_3"));
        assert!(multiline.text.contains("123_comments"));

        assert!(!golf.text_range.is_empty());
        Ok(())
    }

    #[test]
    fn test_unsorted_onlys_multiline_with_comments() -> Result<()> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_fortran::LANGUAGE.into())
            .context("Error loading Fortran grammar")?;

        let code = r#"
program test
  use multiline_module, only: fun_2, & !! fun_2_comments
                              fun_3, & !! fun_3_comments
                              fun_1 !! fun_1_comments
end program test
"#;

        let tree = parser.parse(code, None).context("Failed to parse")?;
        let src = SourceFileBuilder::new("test.f90", code).finish();

        let program_node = tree.root_node().child(0).context("Missing program node")?;
        assert_eq!(program_node.kind(), "program");

        let use_statements: Vec<UseStatementData> = program_node
            .children(&mut program_node.walk())
            .filter(|child| child.kind() == "use_statement")
            .map(|child| extract_use_statement_data(&child, &src))
            .collect();

        assert_eq!(use_statements.len(), 1);
        let use_stmt = &use_statements[0];
        
        assert_eq!(use_stmt.only_items.len(), 3);
        assert_eq!(use_stmt.only_items[0].name, "fun_2");
        assert_eq!(use_stmt.only_items[1].name, "fun_3");
        assert_eq!(use_stmt.only_items[2].name, "fun_1");
        
        // Check that names are extracted correctly for sorting
        let names: Vec<String> = use_stmt.only_items.iter()
            .map(|item| extract_item_name(item))
            .collect();
        assert_eq!(names, vec!["fun_2", "fun_3", "fun_1"]);
        
        // The items should be detected as unsorted
        let mut items_with_names: Vec<(String, OnlyStatementData)> = use_stmt.only_items.iter()
            .map(|item| (extract_item_name(item), item.clone()))
            .collect();
        items_with_names.sort_by(|a, b| a.0.cmp(&b.0));
        
        let sorted_names: Vec<String> = items_with_names.iter()
            .map(|(name, _): &(String, OnlyStatementData)| name.clone())
            .collect();
        assert_eq!(sorted_names, vec!["fun_1", "fun_2", "fun_3"]);
        
        // Check the expected fix result
        let sorted_items: Vec<String> = items_with_names.into_iter()
            .map(|(_, item)| {
                let mut text = item.name.clone();
                if let Some(alias) = &item.alias {
                    text.push_str(" as ");
                    text.push_str(alias);
                }
                if let Some(ref comment) = item.inline_comment {
                    text.push_str(" ");
                    text.push_str(comment);
                }
                text
            })
            .collect();
        let sorted_only = sorted_items.join(", ");
        let before = use_stmt.text.split("only:").nth(0).unwrap();
        let expected_replacement = format!("{}only: {}", before, sorted_only);
        
        // The expected result should contain the items in sorted order
        assert!(expected_replacement.contains("fun_1"));
        assert!(expected_replacement.contains("fun_2"));
        assert!(expected_replacement.contains("fun_3"));
        assert!(expected_replacement.contains("fun_1_comments"));
        assert!(expected_replacement.contains("fun_2_comments"));
        assert!(expected_replacement.contains("fun_3_comments"));

        Ok(())
    }

    #[test]
    fn test_extract_name_and_comment() {
        use crate::rules::style::use_statement::extract_name_and_comment;

        // Test simple item without comment
        let item = extract_name_and_comment("fun_1");
        assert_eq!(item.name, "fun_1");
        assert_eq!(item.alias, None);
        assert_eq!(item.inline_comment, None);

        // Test item with comment
        let item = extract_name_and_comment("fun_1 !! comment");
        assert_eq!(item.name, "fun_1");
        assert_eq!(item.alias, None);
        assert_eq!(item.inline_comment, Some("!! comment".to_string()));

        // Test item with continuation marker
        let item = extract_name_and_comment("fun_1 &");
        assert_eq!(item.name, "fun_1");
        assert_eq!(item.alias, None);
        assert_eq!(item.inline_comment, None);

        // Test item with continuation and comment
        let item = extract_name_and_comment("fun_1 & !! comment");
        assert_eq!(item.name, "fun_1");
        assert_eq!(item.alias, None);
        assert_eq!(item.inline_comment, Some("!! comment".to_string()));

        // Test item with trailing comma
        let item = extract_name_and_comment("fun_1,");
        assert_eq!(item.name, "fun_1");
        assert_eq!(item.alias, None);
        assert_eq!(item.inline_comment, None);

        // Test item with trailing comma and comment
        let item = extract_name_and_comment("fun_1, !! comment");
        assert_eq!(item.name, "fun_1");
        assert_eq!(item.alias, None);
        assert_eq!(item.inline_comment, Some("!! comment".to_string()));

        // Test item with renaming
        let item = extract_name_and_comment("fun_1 as alias");
        assert_eq!(item.name, "fun_1");
        assert_eq!(item.alias, Some("alias".to_string()));
        assert_eq!(item.inline_comment, None);

        // Test item with renaming and comment
        let item = extract_name_and_comment("fun_1 as alias !! comment");
        assert_eq!(item.name, "fun_1");
        assert_eq!(item.alias, Some("alias".to_string()));
        assert_eq!(item.inline_comment, Some("!! comment".to_string()));

        // Test complex case with continuation, comma, and comment
        let item = extract_name_and_comment("fun_1, & !! comment");
        assert_eq!(item.name, "fun_1");
        assert_eq!(item.alias, None);
        assert_eq!(item.inline_comment, Some("!! comment".to_string()));

        // Test empty string
        let item = extract_name_and_comment("");
        assert_eq!(item.name, "");
        assert_eq!(item.alias, None);
        assert_eq!(item.inline_comment, None);

        // Test only comment
        let item = extract_name_and_comment("!! comment");
        assert_eq!(item.name, "");
        assert_eq!(item.alias, None);
        assert_eq!(item.inline_comment, Some("!! comment".to_string()));

        // Test whitespace handling
        let item = extract_name_and_comment("  fun_1  ");
        assert_eq!(item.name, "fun_1");
        assert_eq!(item.alias, None);
        assert_eq!(item.inline_comment, None);

        // Test mixed case (should be lowercased)
        let item = extract_name_and_comment("Fun_1");
        assert_eq!(item.name, "fun_1");
        assert_eq!(item.alias, None);
        assert_eq!(item.inline_comment, None);
    }
}


