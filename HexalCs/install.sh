./build.sh
mkdir -p ~/bin
cp ./Hexal/bin/Release/net9.0/linux-x64/native/hexal ~/bin/hexal
# NLua is backed by the native Lua 5.4 runtime. Keep the shared library next
# to the NativeAOT executable so it is found on startup.
cp ./Hexal/bin/Release/net9.0/linux-x64/liblua54.so ~/bin/liblua54.so
