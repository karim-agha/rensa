const web3 = require('rensa-web3');
const host = process.argv[2];

/**
 * Tests minting & burning coins
 * 
 * This test case ensures that coin dust accounts get automatically reclaimed.
 * More context here: https://www.gemini.com/cryptopedia/basics-of-crypto-dusting 
 */

(async () => {
  const payer = await web3.Keypair.random();
  const wallet = await web3.Keypair.random();

  const client = new web3.Client(host);

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
  console.log("Minting 10.00 coins to a random wallet...");
  console.log(
    await client.sendAndConfirmTransaction(
      await currency.mint(client, wallet.publicKey, payer, 1000)));

  const CURRENCY_CONTRACT_ADDR = new web3.Pubkey("Currency1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx");
  let coinAddress = CURRENCY_CONTRACT_ADDR.derive([mintAddress, wallet.publicKey]);
  console.log("coin address", coinAddress.toString());
  let coinAccount = await client.getAccount(coinAddress);

  if (coinAccount === null) {
    console.error("Failed to create coin account for wallet");
    return;
  } else {
    console.log("coin account", coinAccount);
  }

  let balance = await currency.balance(client, wallet.publicKey);

  if (!balance.eqn(1000)) {
    console.error("test failed: Account balance after mint is expected to be 1000, actual", balance.toString());
    return;
  } else {
    console.log("balance", balance.toString());
  }

  // now burn everything, and the coin account should reflect zero coins
  // 1. first in the confirmed stage it should correctly report zero balance.
  // 2. Then when finalized, zero balance account should be reclaimed and removed from
  //    the accounts database entirely.  
  console.log("Burning 10 coins");
  const burntx = await client.sendAndConfirmTransaction(
    await currency.burn(client, wallet, 1000));
  console.log(burntx);

  console.log("reading updated balance");
  let balanceAfter = await currency.balance(client, wallet.publicKey);
  if (!balanceAfter.eqn(0)) {
    console.error("test failed: Account balance after burn is expected to be 0, actual", balanceAfter.toString());
    return;
  } else {
    console.log("balance after burn", balanceAfter.toString());
  }

  console.log("waiting for burn tx to finalize...");
  await client.confirmTransaction(burntx.hash, web3.Commitment.Finalized);
  let balanceAfterFinalization = await currency.balance(client,
    wallet.publicKey, web3.Commitment.Confirmed);

  if (!balanceAfterFinalization.eqn(0)) {
    console.error("test failed: reclaimed dust account is not reporting zero balance. reporting: ", 
      balanceAfterFinalization.toString());
  }

  let coinAccountAfter = await client.getAccount(coinAddress);
  if (coinAccountAfter !== null) {
    console.error("test failed: dust account not reclaimed");
    return;
  } else {
    console.log("test succeeded, dust account removed");
  }

})();