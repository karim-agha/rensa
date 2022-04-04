class Pubkey { }
class AccountView { }
class InputAccount {
  address: Pubkey;
  view: AccountView;
}
class Environment {
  address: Pubkey;
  accounts: Array<InputAccount>

  constructor() {
    this.address = new Pubkey();
    this.accounts = new Array<InputAccount>(0);
  }
}
class Output { }

function createOutput(output: Output): u32 {
  return changetype<u32>(heap.alloc(sizeof<Output>()));
}

export function allocate(size: u32): u32 {
  return changetype<u32>(heap.alloc(changetype<usize>(size)));
}

export function environment(ptr: u32): Environment {
  return new Environment();
}

export function contract(env: Environment, params: u8[]): u32 {
  return createOutput(new Output());
}
