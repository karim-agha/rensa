create table if not exists block
(
    height     int      not null
        primary key,
    hash       char(46) not null,
    parent     char(46) not null,
    producer   char(44) not null,
    signature  char(88) not null,
    state      char(46) not null,
    commitment char(9)  not null,
    timestamp  datetime not null,
    constraint block_hash_uindex
        unique (hash)
);

create table if not exists state_diff
(
    block   int        not null,
    account char(44)   not null,
    data    text       null,
    deleted tinyint(1) not null,
    `order` int        not null,
    constraint state_diff_block
        foreign key (block) references block (height)
            on delete cascade
);

create table if not exists transaction
(
    hash     char(46)                 not null
        primary key,
    block    int                      not null,
    contract varchar(48) charset utf8 not null,
    nonce    int                      not null,
    params   text                     null,
    payer    char(46)                 not null,
    constraint transaction_hash_uindex
        unique (hash),
    constraint transaction_block
        foreign key (block) references block (height)
            on delete cascade
);

create table if not exists transaction_accounts
(
    transaction char(46)              not null,
    account     char(44) charset utf8 not null,
    signer      tinyint(1)            not null,
    writable    tinyint(1)            not null,
    `order`     int                   not null,
    constraint transaction_accounts_tx_hash
        foreign key (transaction) references transaction (hash)
            on delete cascade
);

create table if not exists transaction_errors
(
    transaction char(46) not null,
    error       text     null,
    constraint transaction_errors_tx
        foreign key (transaction) references transaction (hash)
            on delete cascade
);

create table if not exists transaction_logs
(
    transaction char(46) not null,
    `key`       text     not null,
    value       text     not null,
    `order`     int      not null,
    constraint transaction_logs_tx
        foreign key (transaction) references transaction (hash)
            on delete cascade
);

create table if not exists transaction_signers
(
    transaction char(46) not null,
    signer      char(44) not null,
    signature   char(88) not null,
    `order`     int      not null,
    constraint transaction_signers_transaction_hash
        foreign key (transaction) references transaction_accounts (transaction)
            on delete cascade
);

create table if not exists vote
(
    block         int      not null,
    target        char(46) not null,
    justification char(46) not null,
    validator     char(44) not null,
    signature     char(88) not null,
    constraint vote_block
        foreign key (block) references block (height)
            on delete cascade
);

