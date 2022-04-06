cpout: makecontract
	@cp target/wasm32-unknown-unknown/release/name_service.wasm out/name_service.wasm

makecontract: outdir
	@cargo rustc --release --target wasm32-unknown-unknown -- -C "link-args=-z stack-size=32768"

outdir:
	@mkdir -p out