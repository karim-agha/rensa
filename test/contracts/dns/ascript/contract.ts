class Pubkey { }
class AccountView { }
class InputAccount {
  address: Pubkey;
  view: AccountView;
}
class Environment {
  address: Pubkey;
  accounts: Array<InputAccount>
}
class Output { }

export function contract(env: Environment, b: Uint8Array): Array<Output> | Error {
  return new Error("Not implemented yet");
}
