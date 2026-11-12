//! Generate request types, a typed client and metadata from service declarations.

use std::collections::BTreeSet;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as Tokens;
use quote::{format_ident, quote};
use syn::{FnArg, ItemTrait, LitStr, Pat, ReturnType, TraitItem, Type};

#[proc_macro_attribute]
pub fn rpc_service(args: TokenStream, input: TokenStream) -> TokenStream {
    if !args.is_empty() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "rpc_service takes no arguments",
        )
        .to_compile_error()
        .into();
    }
    expand(syn::parse_macro_input!(input as ItemTrait))
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn docs(attrs: &[syn::Attribute]) -> String {
    attrs
        .iter()
        .filter_map(|attr| {
            if !attr.path().is_ident("doc") {
                return None;
            }
            let syn::Meta::NameValue(value) = &attr.meta else {
                return None;
            };
            let syn::Expr::Lit(value) = &value.value else {
                return None;
            };
            let syn::Lit::Str(value) = &value.lit else {
                return None;
            };
            let line = value.value();
            Some(line.strip_prefix(' ').unwrap_or(&line).to_string())
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn pascal(ident: &syn::Ident) -> String {
    ident
        .to_string()
        .trim_start_matches("r#")
        .split('_')
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().collect::<String>() + chars.as_str()
            })
        })
        .collect()
}

fn expand(mut service: ItemTrait) -> syn::Result<Tokens> {
    if !service.generics.params.is_empty()
        || service.generics.where_clause.is_some()
        || !service.supertraits.is_empty()
        || service.unsafety.is_some()
    {
        return Err(syn::Error::new_spanned(
            &service.ident,
            "RPC service traits cannot be generic, unsafe or have supertraits",
        ));
    }
    let service_docs = docs(&service.attrs);
    let service_name = &service.ident;
    let client_name = format_ident!("{}Client", service_name);
    let visibility = &service.vis;
    let mut metadata = Vec::new();
    let mut client_methods = Vec::new();
    let mut implementations = Vec::new();
    let mut requests = Vec::new();
    let mut mirrors = Vec::new();
    let mut names = BTreeSet::new();
    let mut type_names = BTreeSet::new();

    for item in &mut service.items {
        let TraitItem::Fn(method) = item else {
            return Err(syn::Error::new_spanned(
                item,
                "RPC services only contain method signatures",
            ));
        };
        if method.default.is_some()
            || method.sig.asyncness.is_none()
            || !method.sig.generics.params.is_empty()
            || method.sig.generics.where_clause.is_some()
            || method.sig.constness.is_some()
            || method.sig.unsafety.is_some()
            || method.sig.abi.is_some()
            || method.sig.variadic.is_some()
        {
            return Err(syn::Error::new_spanned(
                &method.sig,
                "expected a non-generic async method declaration without a body",
            ));
        }
        let mut wire_name = None;
        for attr in &method.attrs {
            if attr.path().is_ident("rpc") {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("name") && wire_name.is_none() {
                        wire_name = Some(meta.value()?.parse::<LitStr>()?);
                        Ok(())
                    } else {
                        Err(meta.error("expected a single name = \"namespace.method\""))
                    }
                })?;
            } else if !attr.path().is_ident("doc") && !attr.path().is_ident("deprecated") {
                return Err(syn::Error::new_spanned(
                    attr,
                    "RPC methods support only rpc, doc and deprecated attributes",
                ));
            }
        }
        let wire_name = wire_name.ok_or_else(|| {
            syn::Error::new_spanned(&method.sig, "missing #[rpc(name = \"namespace.method\")]")
        })?;
        if wire_name.value().is_empty() || !names.insert(wire_name.value()) {
            return Err(syn::Error::new_spanned(
                wire_name,
                "RPC names must be non-empty and unique",
            ));
        }
        let ident = &method.sig.ident;
        let type_name = pascal(ident);
        if !type_names.insert(type_name.clone()) {
            return Err(syn::Error::new_spanned(
                ident,
                "generated request type names collide",
            ));
        }
        let params_type = format_ident!("{}Params", type_name);
        let request_type = format_ident!("{}Request", type_name);
        let mut fields = Vec::new();
        let mut args = Vec::new();
        let mut parameters = Vec::new();
        for input in &mut method.sig.inputs {
            let FnArg::Typed(param) = input else {
                return Err(syn::Error::new_spanned(
                    input,
                    "declare RPC arguments without &self, as in tarpc",
                ));
            };
            let Pat::Ident(binding) = param.pat.as_ref() else {
                return Err(syn::Error::new_spanned(
                    &param.pat,
                    "expected a named argument",
                ));
            };
            if binding.by_ref.is_some()
                || binding.mutability.is_some()
                || binding.subpat.is_some()
                || matches!(param.ty.as_ref(), Type::Reference(_) | Type::ImplTrait(_))
            {
                return Err(syn::Error::new_spanned(
                    param,
                    "RPC arguments must be owned named values",
                ));
            }
            if param.attrs.iter().any(|attr| !attr.path().is_ident("doc")) {
                return Err(syn::Error::new_spanned(
                    param,
                    "RPC arguments support only doc comments",
                ));
            }
            let name = &binding.ident;
            let wire_param = name.to_string().trim_start_matches("r#").to_string();
            let ty = &param.ty;
            let attrs = &param.attrs;
            let documentation = docs(attrs);
            fields.push(quote! { #(#attrs)* pub #name: #ty });
            args.push(name.clone());
            parameters.push(quote! {
                ::gui_use_protocol::ParameterSpec {
                    name: #wire_param,
                    rust_type: stringify!(#ty),
                    docs: #documentation,
                }
            });
            // Parameter comments belong to request fields and metadata, not Rust bindings.
            param.attrs.clear();
        }
        let result: Type = match &method.sig.output {
            ReturnType::Default => syn::parse_quote!(()),
            ReturnType::Type(_, ty) => *ty.clone(),
        };
        let documentation = docs(&method.attrs);
        method.attrs.retain(|attr| !attr.path().is_ident("rpc"));
        method.sig.inputs.insert(0, syn::parse_quote!(&self));
        method.sig.output = syn::parse_quote!(-> ::gui_use_protocol::RpcResult<#result>);
        let signature = &method.sig;
        let attrs = &method.attrs;

        mirrors.push(quote! {
            #[typeshare]
            #visibility struct #params_type { #(#fields,)* }
        });
        requests.push(quote! {
            #[derive(Debug, Clone, ::gui_use_protocol::serde::Serialize,
                ::gui_use_protocol::serde::Deserialize, ::gui_use_protocol::schemars::JsonSchema)]
            #[serde(crate = "::gui_use_protocol::serde")]
            #[schemars(crate = "::gui_use_protocol::schemars")]
            #visibility struct #params_type { #(#fields,)* }

            #[derive(Debug, Clone, Copy)]
            #visibility struct #request_type;

            impl ::gui_use_protocol::RpcMethod for #request_type {
                const METHOD: &'static str = #wire_name;
                type Params = #params_type;
                type Result = #result;
            }
        });
        metadata.push(quote! {
            ::gui_use_protocol::MethodSpec {
                ident: stringify!(#ident),
                name: #wire_name,
                params: stringify!(#params_type),
                result: stringify!(#result),
                args: &[#(#parameters),*],
                docs: #documentation,
                schema: || (
                    ::gui_use_protocol::rpc_schema::<#params_type>(),
                    ::gui_use_protocol::rpc_schema::<#result>(),
                ),
            }
        });
        client_methods.push(quote! {
            #(#attrs)*
            pub #signature {
                self.rpc.call::<#request_type>(#params_type { #(#args,)* }).await
            }
        });
        implementations.push(quote! {
            #signature { #client_name::#ident(self, #(#args),*).await }
        });
    }

    let types_source = quote!(#(#mirrors)*).to_string();
    Ok(quote! {
        #[allow(async_fn_in_trait)]
        #service
        #(#requests)*

        #[doc = #service_docs]
        #visibility struct #client_name<T> {
            rpc: ::gui_use_protocol::RpcClient<T>,
        }

        impl<T: ::gui_use_protocol::RpcTransport> #client_name<T> {
            pub fn new(transport: T) -> Self {
                Self { rpc: ::gui_use_protocol::RpcClient::new(transport) }
            }
            #(#client_methods)*
        }

        impl<T: ::gui_use_protocol::RpcTransport> #service_name for #client_name<T> {
            #(#implementations)*
        }

        #visibility const METHODS: &[::gui_use_protocol::MethodSpec] = &[#(#metadata),*];
        #visibility const SERVICE: ::gui_use_protocol::ServiceSpec = ::gui_use_protocol::ServiceSpec {
            name: stringify!(#service_name),
            docs: #service_docs,
            methods: METHODS,
            types_source: #types_source,
        };
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_generate_requests_client_and_in_memory_type_source() {
        let output = expand(syn::parse_quote! {
            /// Device operations.
            pub trait Device {
                /// Launch **apps**.
                ///
                /// ```json
                /// {"package_name": "example"}
                /// ```
                #[rpc(name = "app.launch")]
                async fn app_launch(
                    package_name: String,
                    /// Optional activity.
                    activity: Option<String>,
                ) -> AppLaunchResult;

                #[rpc(name = "ping")]
                async fn ping();
            }
        })
        .unwrap();
        syn::parse2::<syn::File>(output.clone()).unwrap();
        let output = output.to_string();
        for expected in [
            "DeviceClient",
            "AppLaunchRequest",
            "AppLaunchParams",
            "RpcMethod",
            "app.launch",
            "Launch **apps**.",
            "Optional activity.",
            "types_source",
            "PingParams",
        ] {
            assert!(output.contains(expected), "missing {expected}");
        }
        assert!(!output.contains("# [rpc ("));
    }

    #[test]
    fn duplicate_wire_names_are_rejected() {
        let error = expand(syn::parse_quote! {
            trait Device {
                #[rpc(name = "app.list")]
                async fn first() -> Output;
                #[rpc(name = "app.list")]
                async fn second() -> Output;
            }
        })
        .unwrap_err();
        assert!(error.to_string().contains("unique"));
    }

    #[test]
    fn invalid_declarations_are_rejected() {
        for declaration in [
            quote!(
                async fn method(&self) -> Output;
            ),
            quote!(
                async fn method(value: &str) -> Output;
            ),
            quote!(
                async fn method((a, b): (i32, i32)) -> Output;
            ),
            quote!(
                async fn method<T>(value: T) -> Output;
            ),
            quote!(
                fn method() -> Output;
            ),
            quote!(
                async fn method() -> Output {}
            ),
        ] {
            let service = syn::parse2(quote! {
                trait Device { #[rpc(name = "method")] #declaration }
            })
            .unwrap();
            assert!(expand(service).is_err());
        }
    }

    #[test]
    fn colliding_generated_names_are_rejected() {
        let service = syn::parse_quote! {
            trait Device {
                #[rpc(name = "first")]
                async fn app_list();
                #[rpc(name = "second")]
                async fn app__list();
            }
        };
        assert!(expand(service).unwrap_err().to_string().contains("collide"));
    }
}
