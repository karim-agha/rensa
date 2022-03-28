const { Currency, Keypair } = require('rensa-web3');
const web3 = require('rensa-web3');

let host = process.argv[2];

(async () => {
  const payer = await web3.Keypair.random();
  const client = new web3.Client(host);

  console.log("creating new coin...");
  let mintTx = await web3.createTransaction(client,
    Currency.create(
      await (await Keypair.random()).publicKey.bytes,
      payer, 2, null, null));

  let result = await client.sendAndConfirmTransactions([mintTx]);
  console.dir(result, { depth: null });

  let mintAddress = new web3.Pubkey(result[0]["output"]["Ok"]["address"]);
  let currency = new web3.Currency(mintAddress);
  console.log("mint address", mintAddress.toString());

  var walletsA = [];
  var walletsB = [];
  let alternate = false;
  const walletsCount = 1000;
  console.log("generating random wallets", walletsCount);

  for (i = 0; i < walletsCount; i++) {
    walletsA.push(await Keypair.random());
    walletsB.push(await Keypair.random());
  }

  console.log("Minting first coins... (A)");

  let mintTransactionsA = [];
  for (let wallet of walletsA) {
    mintTransactionsA.push(currency.mint(wallet.publicKey, payer, 1000000000));
  }

  console.dir(await client.sendAndConfirmTransactions(
    await web3.createManyTransactions(client, mintTransactionsA)),
    { depth: null, maxArrayLength: null });


  console.log("Minting first coins... (B)");

  let mintTransactionsB = [];
  for (let wallet of walletsB) {
    mintTransactionsB.push(currency.mint(wallet.publicKey, payer, 1000000000));
  }

  console.dir(await client.sendAndConfirmTransactions(
    await web3.createManyTransactions(client, mintTransactionsB)),
    { depth: null, maxArrayLength: null });

  while (true) {
    var fromW, toW;
    if (alternate) {
      fromW = walletsB;
      toW = walletsA;
    } else {
      fromW = walletsA;
      toW = walletsB;
    }

    let txs = [];
    for (var i = 0; i < fromW.length - Math.floor(Math.random() * 100); ++i) {
      let amount = Math.floor(Math.random() * 1000);
      txs.push(currency.transfer(fromW[i], toW[i].publicKey, amount));
    }

    client.sendAndConfirmTransactions(
      await web3.createManyTransactions(client, txs))
      .then((txs) => console.dir(txs, { depth: null, maxArrayLength: null }));
    alternate = !alternate;
  }
})();