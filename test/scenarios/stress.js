const { Currency, Keypair } = require('rensa-web3');
const web3 = require('rensa-web3');

let host = process.argv[2];

(async () => {
  while (true) {
    const payer = await web3.Keypair.random();
    const client = new web3.Client(host);

    console.log("creating new coin...");
    let mintTx = await Currency.create(client,
      await (await Keypair.random()).publicKey.bytes,
      payer,
      2,
      null,
      null);

    let result = await client.sendAndConfirmTransaction(mintTx);
    console.log("new coin created", result);

    let mintAddress = new web3.Pubkey(result["output"]["Ok"]["address"]);
    let currency = new web3.Currency(mintAddress);
    console.log("mint address", mintAddress.toString());

    var walletsGen = [];
    const walletsCount = 1000;
    console.log("generating random wallets", walletsCount);

    for (i = 0; i < walletsCount; i++) {
      walletsGen.push(Keypair.random());
    }
    const wallets = await Promise.all(walletsGen);

    console.log("Minting first coins...");

    for (var i = 0; i < 10; ++i) {
      console.dir(await client.sendAndConfirmTransaction(
        await currency.mint(client, wallets[i].publicKey, payer, 1000000000)
      ), { depth: null });
    }

    // sequencial, nonce-dependent batch
    var factor = walletsCount;
    const sqrt = Math.sqrt(walletsCount);
    for (var iteration = 0; iteration < sqrt; ++iteration) {
      var txs = [];
      const count = Math.floor(Math.pow(iteration, 2) / 2);
      console.log("sending transafers, interation, count:", iteration, count);
      for (var i = 0; i < count; ++i) { // parallel batch
        txs.push(client.sendAndConfirmTransaction(
          await currency.transfer(
            client,
            wallets[i], // from
            wallets[i + count].publicKey, // too
            4000000 * factor) // amount
        ));
      }
      factor /= 2;
      console.dir(await (await Promise.all(txs)).map((tx) => tx.output), { depth: null });
    }
  }
})();