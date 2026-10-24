; P01: the version register and the I/O ports' power-on values. Result at RAM offset 0 (bytes):
;  0: $A10001 version, 1: $A10003 data 1, 2: $A10005 data 2, 3: $A10007 data ext,
;  4: $A10009 ctrl 1, 5: $A1000B ctrl 2, 6: $A1000D ctrl ext, 7: $A1000F txdata 1, 8: $A10011 rxdata 1, 9: $A10013 sctrl 1
    .org $200
    lea     $FF0000,a1
    move.b  $A10001,(a1)+
    move.b  $A10003,(a1)+
    move.b  $A10005,(a1)+
    move.b  $A10007,(a1)+
    move.b  $A10009,(a1)+
    move.b  $A1000B,(a1)+
    move.b  $A1000D,(a1)+
    move.b  $A1000F,(a1)+
    move.b  $A10011,(a1)+
    move.b  $A10013,(a1)+
forever:
    bra.s   forever
