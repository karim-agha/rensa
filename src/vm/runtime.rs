use {
  super::contract::{self, ContractError, Environment},
  borsh::BorshSerialize,
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
      .native::<(WasmPtr<u8>, WasmPtr<u8>, u32), WasmPtr<u8>>()
      .map_err(|e| ContractError::Runtime(e.to_string()))?;

    // deliver contract inputs:
    let env_ptr = self.deliver_environment(env)?;
    let params_ptr = self.deliver_params(params)?;

    // invoke the contract with the instansiated environment
    // object and the raw parameters bytes
    let output_ptr = main_func
      .call(env_ptr, params_ptr, params.len() as u32)
      .map_err(|e| ContractError::Runtime(e.to_string()))?;
    println!("main output: {output_ptr:?}");

    Ok(vec![])
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
  /// copies the input parameter bytes as is then returns a
  /// pointer (offset in wasm memory space) to the params.
  fn deliver_params(
    &self,
    params: &[u8],
  ) -> Result<WasmPtr<u8>, ContractError> {
    let params_ptr = self.allocate(params.len())?;
    self.copy_to_contract_memory(params_ptr, params)?;
    Ok(params_ptr)
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
}

#[derive(Debug, WasmerEnv, Clone)]
struct CallContext {
  #[wasmer(export)]
  memory: LazyInit<Memory>,
}

fn abort(
  cx: &CallContext,
  message: WasmPtr<u8, Array>,
  filename: WasmPtr<u8, Array>,
  line: u32,
  column: u32,
) {
  if let Some(memory) = cx.memory.get_ref() {
    println!(
      "abort called: '{:?}' in {:?} line: {}, column: {}",
      message.get_utf8_string_with_nul(memory),
      filename.get_utf8_string_with_nul(memory),
      line,
      column
    );
  }
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
  use {super::Runtime, crate::vm::contract::Environment, anyhow::Result};

  fn dns_create_name(bytecode: &[u8]) -> Result<()> {
    let runtime = Runtime::new(bytecode)?;

    let env = Environment {
      address: "TestContract1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".parse()?,
      accounts: vec![],
    };

    let params = [0u8; 0];

    let output = runtime.invoke(&env, &params)?;
    println!("output from dns test: {output:?}");
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
      "../../test/contracts/dns/rust/out/name_service.wasm"
    ))
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
