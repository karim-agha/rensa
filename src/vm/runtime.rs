use {
  super::contract::{self, ContractError, Environment},
  wasmer::{imports, Cranelift, Instance, Module, Store, Universal},
};

/// This type represents a WASM execution runtime.
pub struct Runtime {
  instance: Instance,
}

impl Runtime {
  pub fn new(bytecode: &[u8]) -> Result<Self, ContractError> {
    let compiler = Cranelift::default();
    let store = Store::new(&Universal::new(compiler).engine());
    let module = Module::new(&store, bytecode)
      .map_err(|e| ContractError::Runtime(e.to_string()))?;

    let imports = imports! {};
    let instance = Instance::new(&module, &imports)
      .map_err(|e| ContractError::Runtime(e.to_string()))?;

    Ok(Self { instance })
  }

  pub fn invoke(&self, _env: &Environment, _params: &[u8]) -> contract::Result {
    todo!()
  }
}

#[cfg(test)]
mod test {
  use {super::Runtime, anyhow::Result};

  #[test]
  fn dns_create_name() -> Result<()> {
    let _runtime = Runtime::new(include_bytes!(
      "../../test/contracts/dns/rust/out/name_service.wasm"
    ))?;

    Ok(())
  }

  #[test]
  fn dns_release_name() -> Result<()> {
    Ok(())
  }

  #[test]
  fn dns_update_name() -> Result<()> {
    Ok(())
  }
}
