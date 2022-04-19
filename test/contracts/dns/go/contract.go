package main

//export environment
func environment(ptr int) int {
	return 10;
}

//export params
func params(ptr int, len int) int {
	return 15;
}

//export output
func output(obj int) int {
	return 18;
}