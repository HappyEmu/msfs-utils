use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote};
use syn::{
    Attribute, Expr, ExprLit, Fields, ItemStruct, Lit, LitFloat, LitStr, Member, Meta, Token, Type,
    parse_macro_input, punctuated::Punctuated, spanned::Spanned,
};

/// Define typed simulation-object data with a validated wire layout.
#[proc_macro_attribute]
pub fn data_definition(_args: TokenStream, item: TokenStream) -> TokenStream {
    let mut input = parse_macro_input!(item as ItemStruct);
    match expand_data_definition(&mut input, &quote!(::msfs_async)) {
        Ok(output) => output.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Define typed SimConnect client data with a validated wire layout.
#[proc_macro_attribute]
pub fn client_data_definition(_args: TokenStream, item: TokenStream) -> TokenStream {
    let mut input = parse_macro_input!(item as ItemStruct);
    match expand_client_data_definition(&mut input, &quote!(::msfs_async)) {
        Ok(output) => output.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Define typed simulation-object data for the `msfs-sync` facade.
#[proc_macro_attribute]
pub fn sync_data_definition(_args: TokenStream, item: TokenStream) -> TokenStream {
    let mut input = parse_macro_input!(item as ItemStruct);
    match expand_data_definition(&mut input, &quote!(::msfs_sync)) {
        Ok(output) => output.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Define typed client data for the `msfs-sync` facade.
#[proc_macro_attribute]
pub fn sync_client_data_definition(_args: TokenStream, item: TokenStream) -> TokenStream {
    let mut input = parse_macro_input!(item as ItemStruct);
    match expand_client_data_definition(&mut input, &quote!(::msfs_sync)) {
        Ok(output) => output.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

fn expand_data_definition(
    input: &mut ItemStruct,
    root: &proc_macro2::TokenStream,
) -> syn::Result<proc_macro2::TokenStream> {
    validate_struct(input)?;
    let add_repr = validate_repr(&input.attrs)?;
    let copy_derives = generated_copy_derives(&input.attrs)?;
    let struct_name = input.ident.clone();
    let mut definitions = Vec::new();
    let mut assertions = Vec::new();

    for field in &mut input.fields {
        let ty = field.ty.clone();
        let datatype = simconnect_datatype(&ty)?;
        let metadata = take_data_metadata(&mut field.attrs)?;
        let name = metadata.name.ok_or_else(|| {
            syn::Error::new(
                field.span(),
                "a simulation datum requires #[name = \"...\"]",
            )
        })?;
        let unit = metadata.unit.ok_or_else(|| {
            syn::Error::new(
                field.span(),
                "a simulation datum requires #[unit = \"...\"]",
            )
        })?;
        let epsilon = metadata.epsilon.unwrap_or_else(|| {
            Expr::Lit(ExprLit {
                attrs: Vec::new(),
                lit: Lit::Float(LitFloat::new("0.0", Span::call_site())),
            })
        });

        definitions.push(quote! {
            (#name, #unit, (#epsilon) as f32, #root::__sys::#datatype)
        });
        assertions.push(quote! {
            let _: ::core::marker::PhantomData<
                #root::__private::AssertSimConnectDatum<#ty>
            > = ::core::marker::PhantomData;
        });
    }

    let repr = add_repr.then(|| quote!(#[repr(C)]));
    Ok(quote! {
        #repr
        #copy_derives
        #input

        impl #root::DataDefinition for #struct_name {
            const DEFINITIONS: &'static [(
                &'static str,
                &'static str,
                f32,
                #root::__sys::SIMCONNECT_DATATYPE,
            )] = &[#(#definitions),*];
        }

        unsafe impl #root::AsyncDataDefinition for #struct_name {}

        const _: () = {
            #(#assertions)*
        };
    })
}

fn expand_client_data_definition(
    input: &mut ItemStruct,
    root: &proc_macro2::TokenStream,
) -> syn::Result<proc_macro2::TokenStream> {
    validate_struct(input)?;
    let add_repr = validate_repr(&input.attrs)?;
    let copy_derives = generated_copy_derives(&input.attrs)?;
    let struct_name = input.ident.clone();
    let mut definitions = Vec::new();
    let mut async_definitions = Vec::new();
    let mut assertions = Vec::new();

    for (index, field) in input.fields.iter_mut().enumerate() {
        let ty = field.ty.clone();
        reject_bool(&ty)?;
        let epsilon_attribute = take_client_epsilon(&mut field.attrs)?;
        let sdk_type = client_data_sdk_type(&ty);
        if epsilon_attribute.is_some() && sdk_type.is_none() {
            return Err(syn::Error::new(
                ty.span(),
                "#[epsilon] requires a scalar i8/i16/i32/i64/f32/f64 client-data field",
            ));
        }
        let epsilon = epsilon_attribute.unwrap_or_else(|| {
            Expr::Lit(ExprLit {
                attrs: Vec::new(),
                lit: Lit::Float(LitFloat::new("0.0", Span::call_site())),
            })
        });
        let member = field
            .ident
            .clone()
            .map(Member::Named)
            .unwrap_or_else(|| Member::Unnamed(index.into()));

        definitions.push(quote! {
            (
                ::core::mem::offset_of!(#struct_name, #member),
                ::core::mem::size_of::<#ty>(),
                (#epsilon) as f32,
            )
        });
        let sdk_size_or_type =
            sdk_type.unwrap_or_else(|| quote!(::core::mem::size_of::<#ty>() as u32));
        async_definitions.push(quote! {
            (
                ::core::mem::offset_of!(#struct_name, #member),
                ::core::mem::size_of::<#ty>(),
                #sdk_size_or_type,
                (#epsilon) as f32,
            )
        });
        assertions.push(quote! {
            let _: ::core::marker::PhantomData<
                #root::__private::AssertClientDataDatum<#ty>
            > = ::core::marker::PhantomData;
        });
    }

    let repr = add_repr.then(|| quote!(#[repr(C)]));
    Ok(quote! {
        #repr
        #copy_derives
        #input

        impl #root::ClientDataDefinition for #struct_name {
            fn get_definitions() -> ::std::vec::Vec<(usize, usize, f32)> {
                ::std::vec![#(#definitions),*]
            }
        }

        unsafe impl #root::AsyncClientDataDefinition for #struct_name {
            fn async_definitions() -> ::std::vec::Vec<(usize, usize, u32, f32)> {
                ::std::vec![#(#async_definitions),*]
            }
        }

        const _: () = {
            #(#assertions)*
        };
    })
}

fn validate_struct(input: &ItemStruct) -> syn::Result<()> {
    if !input.generics.params.is_empty() || input.generics.where_clause.is_some() {
        return Err(syn::Error::new(
            input.generics.span(),
            "generic data definitions are not supported",
        ));
    }
    if matches!(input.fields, Fields::Unit) {
        return Err(syn::Error::new(
            input.fields.span(),
            "a data definition must contain at least one field",
        ));
    }
    Ok(())
}

/// Return whether the macro needs to add `repr(C)`.
fn validate_repr(attrs: &[Attribute]) -> syn::Result<bool> {
    let mut has_c = false;
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("repr")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("C") {
                has_c = true;
                Ok(())
            } else if meta.path.is_ident("align") {
                let content;
                syn::parenthesized!(content in meta.input);
                let _: Lit = content.parse()?;
                Ok(())
            } else {
                Err(meta.error("only repr(C) and repr(align(...)) are supported"))
            }
        })?;
    }
    Ok(!has_c)
}

fn generated_copy_derives(attrs: &[Attribute]) -> syn::Result<proc_macro2::TokenStream> {
    let mut has_clone = false;
    let mut has_copy = false;
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("derive")) {
        let derives = attr.parse_args_with(Punctuated::<syn::Path, Token![,]>::parse_terminated)?;
        for derive in derives {
            has_clone |= derive.is_ident("Clone");
            has_copy |= derive.is_ident("Copy");
        }
    }

    let missing = [
        (!has_clone).then(|| quote!(Clone)),
        (!has_copy).then(|| quote!(Copy)),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(quote!())
    } else {
        Ok(quote!(#[derive(#(#missing),*)]))
    }
}

fn simconnect_datatype(ty: &Type) -> syn::Result<syn::Ident> {
    let Type::Path(path) = ty else {
        return Err(syn::Error::new(
            ty.span(),
            "unsupported simulation datum type",
        ));
    };
    let Some(segment) = path.path.segments.last() else {
        return Err(syn::Error::new(
            ty.span(),
            "unsupported simulation datum type",
        ));
    };
    let suffix = match segment.ident.to_string().as_str() {
        "i32" => "INT32",
        "i64" => "INT64",
        "f32" => "FLOAT32",
        "f64" => "FLOAT64",
        "DataXYZ" => "XYZ",
        "bool" => {
            return Err(syn::Error::new(
                ty.span(),
                "Rust bool is not wire-safe; use i32 and convert nonzero values to true",
            ));
        }
        _ => {
            return Err(syn::Error::new(
                ty.span(),
                "unsupported simulation datum type; expected i32, i64, f32, f64, or DataXYZ",
            ));
        }
    };
    Ok(format_ident!(
        "SIMCONNECT_DATATYPE_SIMCONNECT_DATATYPE_{suffix}"
    ))
}

fn client_data_sdk_type(ty: &Type) -> Option<proc_macro2::TokenStream> {
    let Type::Path(path) = ty else {
        return None;
    };
    let name = path.path.segments.last()?.ident.to_string();
    let value = match name.as_str() {
        "u8" | "i8" => -1_i32,
        "u16" | "i16" => -2_i32,
        "u32" | "i32" => -3_i32,
        "u64" | "i64" => -4_i32,
        "f32" => -5_i32,
        "f64" => -6_i32,
        _ => return None,
    };
    Some(quote!((#value) as u32))
}

#[derive(Default)]
struct DataMetadata {
    name: Option<LitStr>,
    unit: Option<LitStr>,
    epsilon: Option<Expr>,
}

fn take_data_metadata(attrs: &mut Vec<Attribute>) -> syn::Result<DataMetadata> {
    let mut metadata = DataMetadata::default();
    let mut retained = Vec::with_capacity(attrs.len());
    for attr in attrs.drain(..) {
        if attr.path().is_ident("name") {
            if metadata.name.is_some() {
                return Err(syn::Error::new(attr.span(), "duplicate #[name] attribute"));
            }
            metadata.name = Some(parse_string_attribute(&attr)?);
        } else if attr.path().is_ident("unit") {
            if metadata.unit.is_some() {
                return Err(syn::Error::new(attr.span(), "duplicate #[unit] attribute"));
            }
            metadata.unit = Some(parse_string_attribute(&attr)?);
        } else if attr.path().is_ident("epsilon") {
            if metadata.epsilon.is_some() {
                return Err(syn::Error::new(
                    attr.span(),
                    "duplicate #[epsilon] attribute",
                ));
            }
            metadata.epsilon = Some(parse_number_attribute(&attr)?);
        } else {
            retained.push(attr);
        }
    }
    *attrs = retained;
    Ok(metadata)
}

fn take_client_epsilon(attrs: &mut Vec<Attribute>) -> syn::Result<Option<Expr>> {
    let mut epsilon = None;
    let mut retained = Vec::with_capacity(attrs.len());
    for attr in attrs.drain(..) {
        if attr.path().is_ident("epsilon") {
            if epsilon.is_some() {
                return Err(syn::Error::new(
                    attr.span(),
                    "duplicate #[epsilon] attribute",
                ));
            }
            epsilon = Some(parse_number_attribute(&attr)?);
        } else {
            retained.push(attr);
        }
    }
    *attrs = retained;
    Ok(epsilon)
}

fn parse_string_attribute(attr: &Attribute) -> syn::Result<LitStr> {
    let Meta::NameValue(value) = &attr.meta else {
        return Err(syn::Error::new(
            attr.span(),
            "expected #[attribute = \"value\"]",
        ));
    };
    let Expr::Lit(ExprLit {
        lit: Lit::Str(value),
        ..
    }) = &value.value
    else {
        return Err(syn::Error::new(value.value.span(), "expected a string"));
    };
    Ok(value.clone())
}

fn parse_number_attribute(attr: &Attribute) -> syn::Result<Expr> {
    let Meta::NameValue(value) = &attr.meta else {
        return Err(syn::Error::new(attr.span(), "expected #[epsilon = number]"));
    };
    match &value.value {
        Expr::Lit(ExprLit {
            lit: Lit::Float(_) | Lit::Int(_),
            ..
        }) => Ok(value.value.clone()),
        expression => Err(syn::Error::new(expression.span(), "expected a number")),
    }
}

fn reject_bool(ty: &Type) -> syn::Result<()> {
    if let Type::Path(path) = ty
        && path
            .path
            .segments
            .last()
            .is_some_and(|item| item.ident == "bool")
    {
        return Err(syn::Error::new(
            ty.span(),
            "Rust bool is not safe in externally writable client data; use u8 or i32",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn expands_simulation_data_and_removes_field_attributes() {
        let mut input: ItemStruct = parse_quote! {
            #[derive(Debug)]
            struct Data {
                #[name = "RADIO HEIGHT"]
                #[unit = "Feet"]
                #[epsilon = 0.01]
                height: f64,
            }
        };

        let output = expand_data_definition(&mut input, &quote!(::msfs_async)).unwrap();
        let rendered = output.to_string();
        assert!(rendered.contains("repr (C)"));
        assert!(rendered.contains("AsyncDataDefinition"));
        assert!(input.fields.iter().all(|field| field.attrs.is_empty()));
        syn::parse2::<syn::File>(output).unwrap();
    }

    #[test]
    fn rejects_bool_simulation_data() {
        let mut input: ItemStruct = parse_quote! {
            struct Data {
                #[name = "SIM ON GROUND"]
                #[unit = "Bool"]
                value: bool,
            }
        };

        let error = expand_data_definition(&mut input, &quote!(::msfs_async)).unwrap_err();
        assert!(error.to_string().contains("bool"));
    }

    #[test]
    fn rejects_bool_client_data() {
        let mut input: ItemStruct = parse_quote! {
            struct Data { value: bool }
        };

        let error = expand_client_data_definition(&mut input, &quote!(::msfs_async)).unwrap_err();
        assert!(error.to_string().contains("bool"));
    }

    #[test]
    fn rejects_packed_layouts() {
        let mut input: ItemStruct = parse_quote! {
            #[repr(C, packed)]
            struct Data { value: u32 }
        };

        let error = expand_client_data_definition(&mut input, &quote!(::msfs_async)).unwrap_err();
        assert!(error.to_string().contains("repr(C)"));
    }

    #[test]
    fn preserves_existing_copy_derives() {
        let mut input: ItemStruct = parse_quote! {
            #[derive(Clone, Copy)]
            struct Data {
                #[name = "RADIO HEIGHT"]
                #[unit = "Feet"]
                value: f64,
            }
        };

        let output = expand_data_definition(&mut input, &quote!(::msfs_async)).unwrap();
        assert_eq!(output.to_string().matches("derive").count(), 1);
    }

    #[test]
    fn rejects_duplicate_field_metadata() {
        let mut input: ItemStruct = parse_quote! {
            struct Data {
                #[name = "RADIO HEIGHT"]
                #[name = "PLANE ALTITUDE"]
                #[unit = "Feet"]
                value: f64,
            }
        };

        let error = expand_data_definition(&mut input, &quote!(::msfs_async)).unwrap_err();
        assert!(error.to_string().contains("duplicate #[name]"));
    }

    #[test]
    fn rejects_epsilon_for_untyped_client_data() {
        let mut input: ItemStruct = parse_quote! {
            struct Data {
                #[epsilon = 1]
                bytes: [u8; 4],
            }
        };

        let error = expand_client_data_definition(&mut input, &quote!(::msfs_async)).unwrap_err();
        assert!(error.to_string().contains("requires a scalar"));
    }
}
