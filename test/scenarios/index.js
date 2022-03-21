const { Commitment, Currency, Keypair } = require('rensa-web3');
const web3 = require('rensa-web3');


(async () => {
  const payer = await web3.Keypair.random();
  const client = new web3.Client("http://3.70.190.253:8080");

  console.log("creating new coin...");
  let mintTx = await Currency.create(client,
    payer.publicKey.bytes,
    payer,
    4,
    null,
    null);

  let result = await client.sendAndConfirmTransaction(mintTx);
  console.log("new coin created", result);
})();