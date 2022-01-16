// vote = (
//  validator, 
//  target_block_hash, 
//  target_epoch,    // epoch height
//  source_epoch     // epoch height, has to be justified
// )


// finalized checkpoint when two we have two 
// justified (2/3 majority votes) checkpoints in a row

// slashing conditions:
//
// 1. No two votes from the same validator must have the same
//    target epoch.
// 
// 2. no surround vote.
//      +----------> [h(s1) = 3] ----> [h(t1) = 4] --->
//  [J] +
//      +---> [h(s2) = 2]--------------------------> [h(t2) = 5] ---->

