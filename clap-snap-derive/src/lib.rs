use heck::ToUpperCamelCase;
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use std::fs;
use std::path::{Path, PathBuf};
use syn::{
    DeriveInput, FnArg, ImplItem, Item, ItemImpl, ItemMod, Pat, PatType, Type, parse_macro_input,
};

#[proc_macro_attribute]
pub fn command(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_derive(App)]
pub fn derive_app(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let app_ident = &input.ident;

    let fields = match &input.data {
        syn::Data::Struct(data) => match &data.fields {
            syn::Fields::Named(fields) => fields.named.iter().collect::<Vec<_>>(),
            _ => {
                return quote! {
                    compile_error!("App can only be derived for structs with named fields");
                }
                .into();
            }
        },
        _ => {
            return quote! {
                compile_error!("App can only be derived for structs");
            }
            .into();
        }
    };

    let app_field_idents = fields
        .iter()
        .map(|f| f.ident.as_ref().expect("named field"))
        .collect::<Vec<_>>();
    let app_field_defs = fields
        .iter()
        .map(|f| {
            let attrs = &f.attrs;
            let ident = f.ident.as_ref().expect("named field");
            let ty = &f.ty;
            let has_clap_attr = attrs.iter().any(|attr| {
                let segs = &attr.path().segments;
                segs.last()
                    .is_some_and(|s| s.ident == "arg" || s.ident == "clap")
            });
            let default_attr = if has_clap_attr {
                quote! {}
            } else {
                quote! { #[clap(long)] }
            };
            quote! {
                #(#attrs)*
                #default_attr
                #ident: #ty
            }
        })
        .collect::<Vec<_>>();

    let methods = find_command_methods(app_ident);
    let generated_options_ident = format_ident!("__ClapSnap{}Options", app_ident);
    let generated_command_ident = format_ident!("__ClapSnap{}Command", app_ident);

    let command_option_structs = methods
        .iter()
        .map(|method| {
            let opt_ident = &method.options_ident;
            let arg_defs = method
                .args
                .iter()
                .map(|arg| {
                    let attrs = &arg.attrs;
                    let ident = &arg.ident;
                    let ty = &arg.ty;
                    quote! {
                        #(#attrs)*
                        #ident: #ty
                    }
                })
                .collect::<Vec<_>>();

            quote! {
                #[derive(clap::Args)]
                struct #opt_ident {
                    #(#arg_defs,)*
                }
            }
        })
        .collect::<Vec<_>>();

    let command_variants = methods
        .iter()
        .map(|method| {
            let variant_ident = &method.variant_ident;
            let opt_ident = &method.options_ident;
            quote! { #variant_ident(#opt_ident) }
        })
        .collect::<Vec<_>>();

    let dispatch_arms = methods
        .iter()
        .map(|method| {
            let method_ident = &method.ident;
            let variant_ident = &method.variant_ident;
            let arg_idents = method.args.iter().map(|a| &a.ident).collect::<Vec<_>>();
            quote! {
                #generated_command_ident::#variant_ident(cmd) => {
                    appopts.#method_ident(#(cmd.#arg_idents),*)?;
                }
            }
        })
        .collect::<Vec<_>>();

    quote! {
        #(#command_option_structs)*

        #[derive(clap::Subcommand)]
        enum #generated_command_ident {
            #(#command_variants,)*
        }

        #[derive(clap::Parser)]
        struct #generated_options_ident {
            #(#app_field_defs,)*
            #[clap(subcommand)]
            appcmd: Option<#generated_command_ident>,
        }

        impl clap::FromArgMatches for #app_ident {
            fn from_arg_matches(matches: &clap::ArgMatches) -> Result<Self, clap::Error> {
                let all_opts = <#generated_options_ident as clap::FromArgMatches>::from_arg_matches(matches)?;
                Ok(Self {
                    #(#app_field_idents: all_opts.#app_field_idents,)*
                })
            }

            fn update_from_arg_matches(
                &mut self,
                matches: &clap::ArgMatches,
            ) -> Result<(), clap::Error> {
                let all_opts = <#generated_options_ident as clap::FromArgMatches>::from_arg_matches(matches)?;
                *self = Self {
                    #(#app_field_idents: all_opts.#app_field_idents,)*
                };
                Ok(())
            }
        }

        impl clap::CommandFactory for #app_ident {
            fn command() -> clap::Command {
                <#generated_options_ident as clap::CommandFactory>::command()
            }

            fn command_for_update() -> clap::Command {
                <#generated_options_ident as clap::CommandFactory>::command_for_update()
            }
        }

        impl clap::Parser for #app_ident {}

        impl clap_snap::App for #app_ident {
            fn parse_and_run() -> eyre::Result<()> {
                let mut all_opts = <#generated_options_ident as clap::Parser>::parse();
                let mut appopts = #app_ident {
                    #(#app_field_idents: all_opts.#app_field_idents,)*
                };
                if let Some(cmd) = all_opts.appcmd.take() {
                    match cmd {
                        #(#dispatch_arms)*
                    }
                    Ok(())
                } else {
                    let mut cmd = <#generated_options_ident as clap::CommandFactory>::command();
                    cmd.print_help()?;
                    println!();
                    Ok(())
                }
            }
        }
    }
    .into()
}

#[derive(Clone)]
struct CommandArg {
    attrs: Vec<syn::Attribute>,
    ident: syn::Ident,
    ty: Type,
}

#[derive(Clone)]
struct CommandMethod {
    ident: syn::Ident,
    variant_ident: syn::Ident,
    options_ident: syn::Ident,
    args: Vec<CommandArg>,
}

fn find_command_methods(app_ident: &syn::Ident) -> Vec<CommandMethod> {
    let Some(manifest_dir) = std::env::var_os("CARGO_MANIFEST_DIR") else {
        return Vec::new();
    };

    let src_dir = Path::new(&manifest_dir).join("src");
    if !src_dir.exists() {
        return Vec::new();
    }

    let mut files = Vec::new();
    collect_rs_files(&src_dir, &mut files);

    let mut methods = Vec::new();
    for file in files {
        let contents = fs::read_to_string(&file)
            .unwrap_or_else(|e| {
                panic!(
                    "App derive could not read {} (ensure file permissions are correct): {e}",
                    file.display()
                )
            });
        let parsed = syn::parse_file(&contents).unwrap_or_else(|e| {
            panic!(
                "App derive encountered syntax error in {}: {e}",
                file.display()
            )
        });

        find_methods_in_items(app_ident, &parsed.items, &mut methods);
    }

    methods
}

fn find_methods_in_items(app_ident: &syn::Ident, items: &[Item], methods: &mut Vec<CommandMethod>) {
    for item in items {
        match item {
            Item::Impl(item_impl) => collect_methods_from_impl(app_ident, item_impl, methods),
            Item::Mod(ItemMod {
                content: Some((_, inner_items)),
                ..
            }) => find_methods_in_items(app_ident, inner_items, methods),
            _ => {}
        }
    }
}

fn collect_methods_from_impl(
    app_ident: &syn::Ident,
    item_impl: &ItemImpl,
    methods: &mut Vec<CommandMethod>,
) {
    if item_impl.trait_.is_some() {
        return;
    }

    let Type::Path(self_ty_path) = item_impl.self_ty.as_ref() else {
        return;
    };

    let Some(self_ty_ident) = self_ty_path.path.get_ident() else {
        return;
    };

    if self_ty_ident != app_ident {
        return;
    }

    for impl_item in &item_impl.items {
        let ImplItem::Fn(method) = impl_item else {
            continue;
        };

        if !method.attrs.iter().any(is_command_attr) {
            continue;
        }

        let mut inputs = method.sig.inputs.iter();
        let has_mut_receiver = has_mut_self_receiver(inputs.next());
        if !has_mut_receiver {
            continue;
        }

        let mut args = Vec::new();
        for arg in inputs {
            let FnArg::Typed(PatType { attrs, pat, ty, .. }) = arg else {
                continue;
            };

            let Pat::Ident(pat_ident) = pat.as_ref() else {
                continue;
            };

            args.push(CommandArg {
                attrs: attrs.clone(),
                ident: pat_ident.ident.clone(),
                ty: (*ty.clone()),
            });
        }

        let method_ident = method.sig.ident.clone();
        let variant_ident = format_ident!("{}", method_ident.to_string().to_upper_camel_case());
        let options_ident = format_ident!("__ClapSnap{}{}Options", app_ident, variant_ident);

        methods.push(CommandMethod {
            ident: method_ident,
            variant_ident,
            options_ident,
            args,
        });
    }
}

fn is_command_attr(attr: &syn::Attribute) -> bool {
    let segments = &attr.path().segments;
    segments.last().is_some_and(|seg| seg.ident == "command")
}

fn has_mut_self_receiver(input: Option<&FnArg>) -> bool {
    matches!(
        input,
        Some(FnArg::Receiver(recv)) if recv.reference.is_some() && recv.mutability.is_some()
    )
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}
