const web3 = require('rensa-web3');
const host = process.argv[2];

/**
 * Tests minting and transfering tokens.
 */

(async () => {
  const payer = await web3.Keypair.random();
  const wallet1 = await web3.Keypair.random();
  const wallet2 = await web3.Keypair.random();

  const client = new web3.Client(host);

  console.log("wallet1", wallet1.publicKey.toString());
  console.log("wallet2", wallet2.publicKey.toString());

  console.log("creating new coin...");
  let mintTx = await web3.Currency.create(client,
    await (await web3.Keypair.random()).publicKey.bytes,
    payer,
    2,
    null,
    null);

  let result = await client.sendAndConfirmTransaction(mintTx);
  console.log("new coin created", result);

  let mintAddress = new web3.Pubkey(result["output"]["Ok"]["address"]);
  console.log("mint address", mintAddress.toString());

  let currency = new web3.Currency(mintAddress);
  console.log("Minting 10.00 coins to wallet1...");
  console.log(
    await client.sendAndConfirmTransaction(
      await currency.mint(client, wallet1.publicKey, payer, 1000)));

  let wallet1BalanceAfterMint = await currency.balance(client, wallet1.publicKey);

  if (!wallet1BalanceAfterMint.eqn(1000)) {
    console.error("Invalid balance after mint, expected 1000 actual: ", wallet1BalanceAfterMint);
    return;
  }

  console.log("Transferring 3.00 coins from wallet1 to wallet2...");
  console.log(
    await client.sendAndConfirmTransaction(
      await currency.transfer(client, wallet1, wallet2.publicKey, 300)));

  let wallet1BalanceAfterTransfer = await currency.balance(client, wallet1.publicKey);
  let wallet2BalanceAfterTransfer = await currency.balance(client, wallet2.publicKey);

  if (!wallet1BalanceAfterTransfer.eqn(700)) {
    console.error("Invalid wallet1 balance after transfer, expected 700, actual: ",
      wallet1BalanceAfterTransfer);
    return;
  }

  if (!wallet2BalanceAfterTransfer.eqn(300)) {
    console.error("Invalid wallet1 balance after transfer, expected 300, actual: ",
      wallet1BalanceAfterTransfer);
    return;
  }

  console.log("test succeeded");
})();