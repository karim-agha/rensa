const sdk = require('./sdk');
const bs58 = require('bs58');
const borsh = require('borsh');
const ed = require('@noble/ed25519');

const CURRENCY_CONTRACT_ADDR = new sdk.Pubkey("Currency1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx");


// Creates a new transaction that initiates a new coin type
async function createCoin(payer, seed, decimals, authority, name, symbol) {
  let seedBytes = bs58.decode(seed);
  let mintAddress = CURRENCY_CONTRACT_ADDR.derive([seedBytes]);

  // params
  const writer = new borsh.BinaryWriter();
  writer.writeU8(0);
  writer.writeFixedArray(seedBytes);
  writer.writeFixedArray(authority.bytes);
  writer.writeU8(decimals);
  writer.writeU8(1);
  writer.writeString(name);
  writer.writeU8(1);
  writer.writeString(symbol);

  return await sdk.Transaction.create(
    CURRENCY_CONTRACT_ADDR, // contract
    1, // nonce
    payer, // payer
    [
      {
        address: mintAddress,
        signer: false,
        writable: true
      }
    ], // accounts
    writer.toArray() // params
  );
}

// For an existing coin, mints new coins to a given wallet address
async function mintCoins(payer, coin, to, amount) {

}

// Transfers coins of a given type between two wallets
async function transferCoins(payer, coin, from, to, amount) {

}

(async () => {
  const payer = ed.utils.randomPrivateKey();
  const createcoin = await createCoin(
    payer,
    'csTUBKjVWS4P1Lq5fXQJ1U6JX2dEMef8MFzyNG21ycF',
    2, // decimals
    new sdk.Pubkey('csTUBKjVWS4P1Lq5fXQJ1U6JX2dEMef8MFzyNG21ycF'),
    'Rensa Token',
    'RNS'
  );

  console.log(JSON.stringify(createcoin, null, 2));
})();