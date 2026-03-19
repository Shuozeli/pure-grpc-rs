//! Shared test utilities for codegen tests.

/// Extract all items from the first module in generated code.
pub fn module_items(file: &syn::File) -> &Vec<syn::Item> {
    let module = file.items.iter().find_map(|item| {
        if let syn::Item::Mod(m) = item {
            m.content.as_ref().map(|(_, items)| items)
        } else {
            None
        }
    });
    module.expect("generated code should contain a module")
}
