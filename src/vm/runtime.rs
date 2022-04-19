use {
  super::contract::{self, ContractError, Environment},
  borsh::{BorshDeserialize, BorshSerialize},
  loupe::MemoryUsage,
  std::{ptr::NonNull, sync::Arc},
  wasmer::{
    imports,
    vm::{
      self,
      MemoryStyle,
      TableStyle,
      VMMemoryDefinition,
      VMTableDefinition,
    },
    Array,
    BaseTunables,
    Cranelift,
    Function,
    Instance,
    LazyInit,
    Memory,
    MemoryError,
    MemoryType,
    Module,
    Pages,
    Store,
    TableType,
    Target,
    Tunables,
    Universal,
    WasmPtr,
    WasmerEnv,
  },
};

/// This type represents a WASM execution runtime.
pub struct Runtime {
  instance: Instance,
}

impl Runtime {
  pub fn new(bytecode: &[u8]) -> Result<Self, ContractError> {
    let store = {
      let compiler = Cranelift::default();
      let engine = Universal::new(compiler).engine();
      let base = BaseTunables::for_target(&Target::default());
      let tunables = LimitingTunables::new(base, Pages(4)); // 256 KB of memory
      Store::new_with_tunables(&engine, tunables)
    };

    let module = Module::new(&store, bytecode)
      .map_err(|e| ContractError::Runtime(e.to_string()))?;

    let imports = imports! {
      "env" => {
        "abort" => Function::new_native_with_env(&store, CallContext {
          memory: LazyInit::default()
        }, abort),

        "log" => Function::new_native_with_env(&store, CallContext {
          memory: LazyInit::default()
        }, log)
      }
    };

    let instance = Instance::new(&module, &imports)
      .map_err(|e| ContractError::Runtime(e.to_string()))?;

    Ok(Self { instance })
  }

  pub fn invoke(&self, env: &Environment, params: &[u8]) -> contract::Result {
    // get a function pointer to contract's exported entrypoint
    let main_func = self
      .instance
      .exports
      .get_function("main")
      .map_err(|e| ContractError::Runtime(e.to_string()))?
      .native::<(WasmPtr<u8>, WasmPtr<u8>), WasmPtr<u8>>()
      .map_err(|e| ContractError::Runtime(e.to_string()))?;

    // deliver contract inputs:
    let env_ptr = self.deliver_environment(env)?;
    let params_ptr = self.deliver_params(params)?;

    // invoke the contract with the instansiated environment
    // object and the raw parameters bytes
    let output_ptr = main_func
      .call(env_ptr, params_ptr)
      .map_err(|e| ContractError::Runtime(e.to_string()))?;

    // convert outputs from SDK-format to VM ABI format
    // and return them to the caller. If at any point the
    // contract fails, or returns an error, it is expected
    // to call abort(), and that is already taken care of
    // in the SDK syntatic sugar.
    self.collect_output(output_ptr)
  }

  fn allocate(&self, size: usize) -> Result<WasmPtr<u8>, ContractError> {
    self
      .instance
      .exports
      .get_function("allocate")
      .map_err(|e| ContractError::Runtime(e.to_string()))?
      .native::<u32, WasmPtr<u8>>()
      .map_err(|e| ContractError::Runtime(e.to_string()))?
      .call(size as u32)
      .map_err(|e| ContractError::Runtime(e.to_string()))
  }

  /// This method copies environment inputs to contract's memory and
  /// instanciates it into SDK-specific object. The output of this
  /// method is a pointer in contract's memory space to an object
  /// that is passed to the entrypoint as the environment.
  fn deliver_environment(
    &self,
    env: &Environment,
  ) -> Result<WasmPtr<u8>, ContractError> {
    // to borsh format
    let serialized_env = env
      .try_to_vec()
      .map_err(|e| ContractError::Runtime(e.to_string()))?;

    // allocate and copy to contract address space
    let env_ptr = self.allocate(serialized_env.len())?;
    self.copy_to_contract_memory(env_ptr, &serialized_env[..])?;

    // get the function that instantiates the environment from
    // a raw borsh-serialized byte sequence to SDK-specific object.
    let env_func = self
      .instance
      .exports
      .get_function("environment")
      .map_err(|e| ContractError::Runtime(e.to_string()))?
      .native::<(u32, u32), WasmPtr<u8>>()
      .map_err(|e| ContractError::Runtime(e.to_string()))?;

    // let the contract translate raw borsh env to its native
    // representation. This translation is most likely implemented
    // by the SDK for the target high-level language before it gets
    // compiled to WASM.
    env_func
      .call(env_ptr.offset(), serialized_env.len() as u32)
      .map_err(|e| ContractError::Runtime(e.to_string()))
  }

  /// Allocates memory inside the contract address space and
  /// copies the input parameter bytes then returns a
  /// pointer (offset in wasm memory space) to the params
  /// instantiated in the format of the target sdk.
  fn deliver_params(
    &self,
    params: &[u8],
  ) -> Result<WasmPtr<u8>, ContractError> {
    // get the function that instantiates the params array from
    // a raw borsh-serialized byte sequence to SDK-specific object.
    let params_func = self
      .instance
      .exports
      .get_function("params")
      .map_err(|e| ContractError::Runtime(e.to_string()))?
      .native::<(u32, u32), WasmPtr<u8>>()
      .map_err(|e| ContractError::Runtime(e.to_string()))?;

    let serialized = params.try_to_vec()?;
    let params_ptr = self.allocate(serialized.len())?;
    self.copy_to_contract_memory(params_ptr, &serialized)?;
    params_func
      .call(params_ptr.offset(), serialized.len() as u32)
      .map_err(|e| ContractError::Runtime(e.to_string()))
  }

  fn copy_to_contract_memory(
    &self,
    dst: WasmPtr<u8>,
    src: &[u8],
  ) -> Result<(), ContractError> {
    // get access to contract memory space
    let memory = self
      .instance
      .exports
      .get_memory("memory")
      .map_err(|e| ContractError::Runtime(e.to_string()))?;

    // copy borsh-serialized memory bytes to the allocated
    // memory inside the contract.
    let offset_from = dst.offset() as usize;
    let offset_to = offset_from + src.len();

    unsafe {
      memory.data_unchecked_mut()[offset_from..offset_to] // inside wasm addr space
        .copy_from_slice(src); // copy contents of src from host to contract
    }

    Ok(())
  }

  fn collect_output(
    &self,
    ptr: WasmPtr<u8>,
  ) -> Result<Vec<contract::Output>, ContractError> {
    let output_func = self
      .instance
      .exports
      .get_function("output")
      .map_err(|e| ContractError::Runtime(e.to_string()))?
      .native::<u32, u64>()
      .map_err(|e| ContractError::Runtime(e.to_string()))?;

    // let the SDK convert from SDK's representation of an
    // output to VM representation of an output
    let output_region = output_func
      .call(ptr.offset())
      .map_err(|e| ContractError::Runtime(e.to_string()))?;

    // decode "Region"
    let addr = output_region >> 32;
    let len = (output_region << 32) >> 32;

    // get access to contract memory space
    let memory = self
      .instance
      .exports
      .get_memory("memory")
      .map_err(|e| ContractError::Runtime(e.to_string()))?;

    let start = addr as usize;
    let end = start + len as usize;

    if end >= memory.data_size() as usize {
      return Err(ContractError::Runtime("Invalid outputs format".to_string()));
    }

    // deserialize borsh output representation
    // into VM output objects
    unsafe {
      Vec::<contract::Output>::try_from_slice(
        &memory.data_unchecked()[start..end],
      )
      .map_err(|e| ContractError::Runtime(e.to_string()))
    }
  }
}

#[derive(Debug, WasmerEnv, Clone)]
struct CallContext {
  #[wasmer(export)]
  memory: LazyInit<Memory>,
}

fn abort(cx: &CallContext, region: u64) -> Result<(), ContractError> {
  let addr = region >> 32;
  let len = (region << 32) >> 32;

  let start = addr as usize;
  let end = start + len as usize;

  if let Some(memory) = cx.memory.get_ref() {
    if end >= memory.data_size() as usize {
      return Err(ContractError::Runtime("Invalid error format".to_string()));
    }

    return Err(unsafe {
      ContractError::try_from_slice(&memory.data_unchecked()[start..end])
        .map_err(|e| ContractError::Runtime(e.to_string()))
    }?);
  }

  Err(ContractError::Runtime("inaccessible memory".into()))
}

fn log(cx: &CallContext, str: WasmPtr<u8, Array>) {
  if let Some(memory) = cx.memory.get_ref() {
    if let Some(message) = str.get_utf8_string_with_nul(memory) {
      println!("contract log: {}", message);
    }
  }
}

/// A custom tunables that allows you to set a memory limit.
///
/// After adjusting the memory limits, it delegates all other logic
/// to the base tunables.
#[derive(Clone, MemoryUsage)]
pub struct LimitingTunables<T: Tunables> {
  /// The maximum a linear memory is allowed to be (in Wasm pages, 64 KiB
  /// each). Since Wasmer ensures there is only none or one memory, this is
  /// practically an upper limit for the guest memory.
  limit: Pages,
  /// The base implementation we delegate all the logic to
  base: T,
}

impl<T: Tunables> LimitingTunables<T> {
  pub fn new(base: T, limit: Pages) -> Self {
    Self { limit, base }
  }

  /// Takes an input memory type as requested by the guest and sets
  /// a maximum if missing. The resulting memory type is final if
  /// valid. However, this can produce invalid types, such that
  /// validate_memory must be called before creating the memory.
  fn adjust_memory(&self, requested: &MemoryType) -> MemoryType {
    let mut adjusted = *requested;
    adjusted.maximum = Some(self.limit);
    adjusted
  }

  /// Ensures the a given memory type does not exceed the memory limit.
  /// Call this after adjusting the memory.
  fn validate_memory(&self, ty: &MemoryType) -> Result<(), MemoryError> {
    if ty.minimum > self.limit {
      return Err(MemoryError::Generic(format!(
        "Minimum {} exceeds the allowed memory limit {}",
        ty.minimum.0, self.limit.0
      )));
    }

    if let Some(max) = ty.maximum {
      if max > self.limit {
        return Err(MemoryError::Generic(
          "Maximum exceeds the allowed memory limit".to_string(),
        ));
      }
    } else {
      return Err(MemoryError::Generic("Maximum unset".to_string()));
    }

    Ok(())
  }
}

impl<T: Tunables> Tunables for LimitingTunables<T> {
  /// Construct a `MemoryStyle` for the provided `MemoryType`
  ///
  /// Delegated to base.
  fn memory_style(&self, _memory: &MemoryType) -> MemoryStyle {
    // let adjusted = self.adjust_memory(memory);
    // self.base.memory_style(&adjusted)
    MemoryStyle::Static {
      bound: self.limit,
      offset_guard_size: 0x8000_0000,
    }
  }

  /// Construct a `TableStyle` for the provided `TableType`
  ///
  /// Delegated to base.
  fn table_style(&self, table: &TableType) -> TableStyle {
    self.base.table_style(table)
  }

  /// Create a memory owned by the host given a [`MemoryType`] and a
  /// [`MemoryStyle`].
  ///
  /// The requested memory type is validated, adjusted to the limited and then
  /// passed to base.
  fn create_host_memory(
    &self,
    ty: &MemoryType,
    style: &MemoryStyle,
  ) -> Result<Arc<dyn vm::Memory>, MemoryError> {
    let adjusted = self.adjust_memory(ty);
    self.validate_memory(&adjusted)?;
    self.base.create_host_memory(&adjusted, style)
  }

  /// Create a memory owned by the VM given a [`MemoryType`] and a
  /// [`MemoryStyle`].
  ///
  /// Delegated to base.
  unsafe fn create_vm_memory(
    &self,
    ty: &MemoryType,
    style: &MemoryStyle,
    vm_definition_location: NonNull<VMMemoryDefinition>,
  ) -> Result<Arc<dyn vm::Memory>, MemoryError> {
    let adjusted = self.adjust_memory(ty);
    self.validate_memory(&adjusted)?;
    self
      .base
      .create_vm_memory(&adjusted, style, vm_definition_location)
  }

  /// Create a table owned by the host given a [`TableType`] and a
  /// [`TableStyle`].
  ///
  /// Delegated to base.
  fn create_host_table(
    &self,
    ty: &TableType,
    style: &TableStyle,
  ) -> Result<Arc<dyn vm::Table>, String> {
    self.base.create_host_table(ty, style)
  }

  /// Create a table owned by the VM given a [`TableType`] and a [`TableStyle`].
  ///
  /// Delegated to base.
  unsafe fn create_vm_table(
    &self,
    ty: &TableType,
    style: &TableStyle,
    vm_definition_location: NonNull<VMTableDefinition>,
  ) -> Result<Arc<dyn vm::Table>, String> {
    self.base.create_vm_table(ty, style, vm_definition_location)
  }
}

#[cfg(test)]
mod test {
  use {
    super::Runtime,
    crate::{primitives::Pubkey, vm::contract::Environment},
    anyhow::Result,
    borsh::BorshSerialize,
    std::str::FromStr,
  };

  #[derive(Debug, BorshSerialize)]
  enum Instruction {
    Register { name: String, owner: Pubkey },
    Update { name: String, owner: Pubkey },
    Release { name: String },
  }

  fn dns_create_name(bytecode: &[u8]) -> Result<()> {
    let runtime = Runtime::new(bytecode)?;

    let env = Environment {
      caller: None,
      address: "TestDns1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".parse()?,
      accounts: vec![],
    };

    let params = Instruction::Register {
      name: "example.com".to_owned(),
      owner: crate::primitives::Pubkey::from_str(
        "TestDns1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
      )
      .unwrap(),
    };

    let output = runtime.invoke(&env, &params.try_to_vec().unwrap())?;
    println!("output from dns test: {output:?}");
    Ok(())
  }

  fn dns_release_name(bytecode: &[u8]) -> Result<()> {
    let runtime = Runtime::new(bytecode)?;

    let env = Environment {
      caller: None,
      address: "TestDns1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".parse()?,
      accounts: vec![],
    };

    let params = Instruction::Release {
      name: "example.com".to_owned(),
    };

    let output = runtime.invoke(&env, &params.try_to_vec().unwrap())?;
    println!("output from release test: {output:?}");
    Ok(())
  }

  fn dns_update_name(bytecode: &[u8]) -> Result<()> {
    let runtime = Runtime::new(bytecode)?;

    let env = Environment {
      caller: None,
      address: "TestDns1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".parse()?,
      accounts: vec![],
    };

    let params = Instruction::Update {
      name: "example.com".to_owned(),
      owner: crate::primitives::Pubkey::from_str(
        "TestAccount2xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
      )
      .unwrap(),
    };

    let output = runtime.invoke(&env, &params.try_to_vec().unwrap())?;
    println!("output from dns update test: {output:?}");
    Ok(())
  }

  #[test]
  fn dns_create_name_ascript() -> Result<()> {
    dns_create_name(include_bytes!(
      "../../test/contracts/dns/ascript/build/release.wasm"
    ))
  }

  #[test]
  fn dns_create_name_rust() -> Result<()> {
    dns_create_name(include_bytes!(
      "../../test/contracts/dns/rust/out/release.wasm"
    ))
  }

  #[test]
  fn dns_release_name_ascript() -> Result<()> {
    dns_release_name(include_bytes!(
      "../../test/contracts/dns/ascript/build/release.wasm"
    ))
  }

  #[test]
  fn dns_release_name_rust() -> Result<()> {
    dns_release_name(include_bytes!(
      "../../test/contracts/dns/rust/out/release.wasm"
    ))
  }

  #[test]
  fn dns_update_name_ascript() -> Result<()> {
    dns_update_name(include_bytes!(
      "../../test/contracts/dns/ascript/build/release.wasm"
    ))
  }

  #[test]
  fn dns_update_name_rust() -> Result<()> {
    dns_update_name(include_bytes!(
      "../../test/contracts/dns/rust/out/release.wasm"
    ))
  }
}
