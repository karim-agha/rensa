import { BorshSerializer } from '@serial-as/borsh'

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

export function allocate(size: u32): u32 {
  return changetype<u32>(heap.alloc(changetype<usize>(size)));
}

export function environment(ptr: u32, len: u32): Environment {
  return new Environment();
}

export function output(ptr: u32): u64 {
  let data = BorshSerializer.encode(new Uint8Array(0));
  let addr = changetype<u64>(data);
  let length = data.byteLength as u64;
  return (addr << 32) | length;
}

export function params(ptr: u32, len: u32): Uint8Array {
  return new Uint8Array(len);
}

export function main(env: Environment, params: Uint8Array): Array<Output> {
  return new Array<Output>(0);
}
