# iroh dag cbor example


## Building

```
> cargo build
```

## Usage

In one terminal

```
> ./target/debug/iroh-cbor server
serving <cid>
Listening addresses:
        <addr>
PeerID: <peerid>
Auth token: <auth-token>
```

In another terminal

```
> ./target debug iroh-cbor client <cid> <addr> <peerid> <auth-token>
```
