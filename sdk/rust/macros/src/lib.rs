use {
  proc_macro::TokenStream,
  quote::quote,
  syn::{parse_macro_input, parse_quote, Abi, FnArg, ItemFn},
};

#[proc_macro_attribute]
pub fn main(_input: TokenStream, annotated_item: TokenStream) -> TokenStream {
  let mut input_fn = parse_macro_input!(annotated_item as ItemFn);

  // first, decorate and export the entrypoint function
  decorate_entrypoint_abi(&mut input_fn);

  // make idiomatic rust types ABI and FFI friendly
  adjust_entrypoint_params(&mut input_fn);

  // then transalate Rust-style error handling to an ABI firendly
  // representation of return pointer and abort() or error
  input_fn = wrap_entrypoint_abi(input_fn);

  TokenStream::from(quote!(#input_fn))
}

/// this is the main function of a contract, decorate
/// it with extern and no_mangle and rename it to "main".
/// this is expected by the VM ABI.
fn decorate_entrypoint_abi(input_fn: &mut ItemFn) {
  input_fn.attrs.push(parse_quote! {
    #[no_mangle]
  });
  input_fn.sig.abi = Some(Abi {
    extern_token: parse_quote!(extern),
    name: None,
  });
  input_fn.sig.ident = parse_quote!(main);
  input_fn.sig.output = parse_quote! ( -> Box<Vec<rensa_sdk::Output>> );
}

fn adjust_entrypoint_params(input_fn: &mut ItemFn) {
  if let Some(FnArg::Typed(fn_pat)) = input_fn.sig.inputs.iter_mut().nth(1) {
    fn_pat.ty = Box::new(parse_quote!(&Vec<u8>))
  } else {
    quote! {
      compile_error!(
        "contract entrypoint function is expected have the signature of \
         fn(&Environment, &[u8]) -> Result<Vec<Output>, ContractError>"
      )
    };
  }
}

fn wrap_entrypoint_abi(mut input_fn: ItemFn) -> ItemFn {
  // the original body of the function in idomatic rust
  let body = input_fn.block;

  // wrap it in FFI-compatible pointer on success
  // and on failure call the VM-exported [`abort`]
  // with the error value
  input_fn.block = Box::new(parse_quote!({
    let output = move || -> Result<Vec<rensa_sdk::Output>, rensa_sdk::ContractError> {
      #body
    };
    match output() {
      Ok(result) => Box::new(result),
      Err(error) => rensa_sdk::abort(error)
    }
  }));

  input_fn
}
