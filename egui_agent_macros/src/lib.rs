//! Proc macros for `egui_agent`.

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Fields};

/// Derive `egui_agent::ToSnapshot` for a struct with named fields.
///
/// Every field becomes a snapshot node whose id is the field name and whose
/// value is the field serialized with serde. Skip fields with
/// `#[snapshot(skip)]`; override a node id with `#[snapshot(id = "...")]`.
///
/// ```ignore
/// #[derive(egui_agent_macros::Snapshot)]
/// struct MyState {
///     count: u32,
///     #[snapshot(id = "user_name")]
///     name: String,
///     #[snapshot(skip)]
///     cache: Vec<u8>,
/// }
/// ```
#[proc_macro_derive(Snapshot, attributes(snapshot))]
pub fn derive_snapshot(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let Data::Struct(data) = &input.data else {
        return syn::Error::new_spanned(&input.ident, "#[derive(Snapshot)] only supports structs")
            .to_compile_error()
            .into();
    };
    let Fields::Named(fields) = &data.fields else {
        return syn::Error::new_spanned(
            &input.ident,
            "#[derive(Snapshot)] only supports structs with named fields",
        )
        .to_compile_error()
        .into();
    };

    let mut nodes = Vec::new();
    for field in &fields.named {
        let ident = field.ident.as_ref().unwrap();
        let mut id = ident.to_string();
        let mut skip = false;
        for attr in &field.attrs {
            if !attr.path().is_ident("snapshot") {
                continue;
            }
            let parsed = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("skip") {
                    skip = true;
                    Ok(())
                } else if meta.path.is_ident("id") {
                    let lit: syn::LitStr = meta.value()?.parse()?;
                    id = lit.value();
                    Ok(())
                } else {
                    Err(meta.error("expected `skip` or `id = \"...\"`"))
                }
            });
            if let Err(e) = parsed {
                return e.to_compile_error().into();
            }
        }
        if skip {
            continue;
        }
        nodes.push(quote! {
            nodes.push(
                ::egui_agent::Node::new(::egui_agent::roles::VALUE)
                    .with_id(#id)
                    .with_value(
                        ::egui_agent::serde_json::to_value(&self.#ident)
                            .unwrap_or(::egui_agent::serde_json::Value::Null),
                    ),
            );
        });
    }

    quote! {
        impl #impl_generics ::egui_agent::ToSnapshot for #name #ty_generics #where_clause {
            fn to_snapshot_nodes(&self) -> ::std::vec::Vec<::egui_agent::Node> {
                let mut nodes = ::std::vec::Vec::new();
                #(#nodes)*
                nodes
            }
        }
    }
    .into()
}
