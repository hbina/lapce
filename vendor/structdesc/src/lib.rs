use darling::{ast::Data, FromDeriveInput, FromField};
use proc_macro2::TokenStream;
use quote::{quote, ToTokens};
use syn::{parse_macro_input, Attribute, DeriveInput, Generics, Ident, Type};

#[derive(FromDeriveInput)]
#[darling(supports(struct_named))]
pub(crate) struct Receiver {
    ident: Ident,
    generics: Generics,
    data: Data<(), ReceiverField>,
}

#[derive(FromField, Debug)]
#[darling(attributes(field_names))]
struct ReceiverField {
    ident: Option<Ident>,
    #[darling(default)]
    skip: bool,
    #[darling(default)]
    desc: String,
}

impl ReceiverField {
    fn name(&self) -> String {
        self.ident
            .as_ref()
            .expect("FieldNames only supports named fields")
            .to_string()
    }
}

impl ToTokens for Receiver {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let ident = &self.ident;
        let (impl_generics, ty_generics, where_clause) = self.generics.split_for_impl();
        let fields = self.fields_to_emit();
        let descs = self.descs();
        let fields_len = fields.len();

        tokens.extend(quote! {
            #[automatically_derived]
            impl #impl_generics #ident #ty_generics #where_clause {
                pub const FIELDS: [&'static str; #fields_len] = [
                    #(#fields),*
                ];
                pub const DESCS: [&'static str; #fields_len] = [
                    #(#descs),*
                ];
            }
        })
    }
}

impl Receiver {
    fn fields_to_emit(&self) -> Vec<String> {
        self.data
            .as_ref()
            .take_struct()
            .expect("FieldNames only supports named structs")
            .into_iter()
            .filter(|field| !field.skip)
            .map(|field| field.name())
            .collect()
    }

    fn descs(&self) -> Vec<String> {
        self.data
            .as_ref()
            .take_struct()
            .expect("FieldNames only supports named structs")
            .into_iter()
            .filter(|field| !field.skip)
            .map(|field| field.desc.clone())
            .collect()
    }
}

#[proc_macro_derive(FieldNames, attributes(field_names))]
pub fn derive_field_names(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    Receiver::from_derive_input(&parse_macro_input!(input as DeriveInput))
        .map(|receiver| quote!(#receiver))
        .unwrap_or_else(|err| err.write_errors())
        .into()
}

#[cfg(test)]
mod tests {
    use super::Receiver;
    use darling::FromDeriveInput;
    use syn::parse_quote;

    #[test]
    fn simple() {
        let input = Receiver::from_derive_input(&parse_quote! {
            #[derive(FieldNames)]
            struct Example {
                hello: String,
                world: String,
            }
        })
        .unwrap();

        assert_eq!(
            input.fields_to_emit(),
            vec!["hello".to_string(), "world".to_string()]
        );
    }

    #[test]
    fn skip_field() {
        let input = Receiver::from_derive_input(&parse_quote! {
            #[derive(FieldNames)]
            struct Example {
                hello: String,
                #[field_names(skip)]
                hidden: bool,
                world: String,
            }
        })
        .unwrap();

        assert_eq!(
            input.descs(),
            vec!["hello".to_string(), "world".to_string()]
        );
    }
}
