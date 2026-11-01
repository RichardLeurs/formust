program DelphiDemo;

{$APPTYPE CONSOLE}

uses
  Greeter in 'src\Greeter.pas';

var
  Name: string;
begin
  Name := 'Delphi';
  Writeln(Greeting(Name));
  Writeln('Built by Pascoda from DelphiDemo.dproj');
end.
