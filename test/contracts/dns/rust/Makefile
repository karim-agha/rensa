cpout: makecontract
	@cp target/wasm32-unknown-unknown/release/name_service.wasm out/name_service.wasm

makecontract: outdir
	@cargo build --release --target wasm32-unknown-unknown

outdir:
	@mkdir -p out