# Rensa Blockchain

Documentation and more detailed overview is still in the works. Meanwile to start a test environment that runs the latest progress of work run: 

```
$ docker-compose up --build
```

To run web3 tests that send transactions to the chain through RPC run:
```
$ cd client/js
$ npx tsc
```
then 
```
$ cd test/scenarios
$ node index.js http://<public-ip>:8080
```
where `<public-ip>` is the IP through which services in the docker container can be reached. This IP can be checked by pointing a web browser to `http://<public-ip>:8080/info` and checking if you get a JSON response with info about the running chain.